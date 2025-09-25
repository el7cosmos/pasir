use std::ffi::c_void;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use ext_php_rs::zend::SapiGlobals;
use hyper::HeaderMap;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::Version;
use hyper::body::Frame;
use hyper::header::IntoHeaderName;
use hyper::http::HeaderValue;
use hyper::http::response::Parts;
use pasir::unbound_channel::Sender;
use pasir::unbound_channel::UnboundChannel;
use tokio::sync::oneshot::Receiver;
use tokio::sync::oneshot::Sender as OneShotSender;
use tracing::debug;
use tracing::instrument;

use crate::cli::serve::Stream;
use crate::sapi::ext::FromSapiHeaders;
use crate::sapi::util::handle_abort_connection;
use crate::service::php::PhpRoute;

#[derive(Clone, Debug, Default)]
pub(crate) enum ResponseType {
  #[default]
  Full,
  Chunked,
}

#[derive(Debug)]
pub(crate) struct Context {
  root: Arc<PathBuf>,
  route: PhpRoute,
  stream: Arc<Stream>,
  request: Request<Bytes>,
  response_head: Parts,
  sender: ContextSender,
  request_finished: bool,
}

impl Context {
  pub(crate) fn new(
    root: Arc<PathBuf>,
    route: PhpRoute,
    stream: Arc<Stream>,
    request: Request<Bytes>,
    sender: ContextSender,
  ) -> Self {
    let (response_head, _) = Response::<Bytes>::default().into_parts();
    Self { root, route, stream, request, sender, response_head, request_finished: false }
  }

  #[must_use = "losing the pointer will leak memory"]
  pub(crate) fn into_raw(self) -> *mut Context {
    Box::into_raw(Box::new(self))
  }

  #[must_use = "call `drop(Context::from_raw(ptr))` if you intend to drop the `Context`"]
  pub(crate) unsafe fn from_raw(ptr: *mut c_void) -> Box<Self> {
    unsafe { Box::from_raw(ptr.cast()) }
  }

  pub(crate) fn from_server_context<'a>(server_context: *mut c_void) -> &'a mut Context {
    let context = server_context.cast();
    unsafe { &mut *context }
  }

  pub(crate) fn root(&self) -> &Path {
    self.root.as_path()
  }

  pub(crate) fn route(&self) -> &PhpRoute {
    &self.route
  }

  pub(crate) fn local_addr(&self) -> SocketAddr {
    self.stream.local_addr()
  }

  pub(crate) fn peer_addr(&self) -> SocketAddr {
    self.stream.peer_addr()
  }

  pub(crate) fn headers(&self) -> &HeaderMap {
    self.request.headers()
  }

  pub(crate) fn version(&self) -> Version {
    self.request.version()
  }

  pub(crate) fn body_mut(&mut self) -> &mut Bytes {
    self.request.body_mut()
  }

  pub(crate) fn append_response_header<K>(&mut self, key: K, value: HeaderValue)
  where
    K: IntoHeaderName,
  {
    self.response_head.headers.append(key, value);
  }

  #[instrument(skip(self, data))]
  pub(crate) fn ub_write(&mut self, data: Bytes) -> bool {
    if let Some(mut body_tx) = self.sender.body.take() {
      if let Err(frame) = body_tx.send(Frame::data(data)) {
        debug!("Failed to send data to body channel: {frame}");
        return false;
      }

      self.sender.body = Some(body_tx);
      return true;
    };

    false
  }

  #[instrument(skip(self))]
  pub(crate) fn flush(&mut self) -> bool {
    if self.sender.head.is_some() {
      let mut head = self.response_head.clone();
      head.extensions.insert(ResponseType::Chunked);
      self.sender.send_head(head);
      return true;
    }

    false
  }

  pub(crate) fn is_request_finished(&self) -> bool {
    self.request_finished
  }

  #[instrument(skip(self))]
  pub(crate) fn finish_request(&mut self) -> bool {
    if self.request_finished {
      return false;
    }

    unsafe { ext_php_rs::ffi::php_output_end_all() }

    if let Some(body_tx) = self.sender.body.take() {
      body_tx.abort();
    }
    self.sender.send_head(self.response_head.clone());

    self.request_finished = true;
    true
  }
}

impl Drop for Context {
  fn drop(&mut self) {
    if !SapiGlobals::get().server_context.is_null() {
      SapiGlobals::get_mut().server_context = std::ptr::null_mut();
    }
  }
}

type ContextReceiver = (Receiver<Parts>, UnboundChannel<Bytes>, ContextSender);

#[derive(Default, Debug)]
pub(crate) struct ContextSender {
  head: Option<OneShotSender<Parts>>,
  body: Option<Sender<Bytes>>,
}

impl ContextSender {
  pub(crate) fn receiver() -> ContextReceiver {
    let (head_tx, head_rx) = tokio::sync::oneshot::channel();
    let (body_tx, body_rx) = UnboundChannel::<Bytes>::new();
    (head_rx, body_rx, Self { head: Some(head_tx), body: Some(body_tx) })
  }

  #[instrument(skip(self))]
  pub(crate) fn send_head(&mut self, mut headers: Parts) {
    if let Some(head_tx) = self.head.take() {
      if let Ok(status) = StatusCode::from_sapi_headers(SapiGlobals::get().sapi_headers()) {
        headers.status = status;
      }
      if head_tx.send(headers).is_err() {
        handle_abort_connection();
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use std::net::IpAddr;
  use std::net::Ipv4Addr;
  use std::net::SocketAddr;
  use std::path::PathBuf;
  use std::sync::Arc;

  use bytes::Bytes;
  use ext_php_rs::zend::SapiGlobals;
  use hyper::Request;

  use crate::cli::serve::Stream;
  use crate::sapi::context::Context;
  use crate::sapi::context::ContextSender;
  use crate::sapi::tests::TestSapi;
  use crate::service::php::PhpRoute;

  #[test]
  fn test_context_flush() {
    let _sapi = TestSapi::new();

    let socket = SocketAddr::new(IpAddr::from(Ipv4Addr::LOCALHOST), Default::default());
    let root = Arc::new(PathBuf::default());
    let route = PhpRoute::default();
    let stream = Arc::new(Stream::new(socket, socket));
    let request = Request::new(Bytes::default());

    let (_head_rx, _body_rx, context_sender) = ContextSender::receiver();

    let context = Context::new(root.clone(), route, stream, request, context_sender);
    SapiGlobals::get_mut().server_context = context.into_raw().cast();

    let mut context = unsafe { Context::from_raw(SapiGlobals::get().server_context) };

    // assert that `flush` is true if the request not finished yet.
    assert!(context.flush());
    // assert that `finish_request` is true if the request not finished yet.
    assert!(context.finish_request());
    // assert that `flush` is false if the request finished already.
    assert!(!context.flush());
  }
}
