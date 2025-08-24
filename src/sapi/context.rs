use crate::Stream;
use crate::sapi::ext::FromSapiHeaders;
use crate::sapi::util::handle_abort_connection;
use crate::service::php::PhpRoute;
use bytes::Bytes;
use ext_php_rs::ffi::php_output_end_all;
use ext_php_rs::zend::SapiGlobals;
use hyper::body::Frame;
use hyper::header::IntoHeaderName;
use hyper::http::HeaderValue;
use hyper::http::response::Parts;
use hyper::{HeaderMap, Request, Response, StatusCode, Version};
use pasir::unbound_channel::{Sender, UnboundChannel};
use std::ffi::c_void;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot::{Receiver, Sender as OneShotSender};
use tracing::{debug, instrument};

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

  pub(crate) fn from_server_context<'a>(server_context: *mut c_void) -> &'a mut Context {
    let context = server_context.cast::<Self>();
    unsafe { &mut *context }
  }

  pub(crate) fn root(&self) -> &Path {
    self.root.as_path()
  }

  pub(crate) fn route(&self) -> &PhpRoute {
    &self.route
  }

  pub(crate) fn local_addr(&self) -> SocketAddr {
    self.stream.local_addr
  }

  pub(crate) fn peer_addr(&self) -> SocketAddr {
    self.stream.peer_addr
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

    unsafe { php_output_end_all() }

    if let Some(body_tx) = self.sender.body.take() {
      body_tx.abort();
    }
    self.sender.send_head(self.response_head.clone());

    self.request_finished = true;
    true
  }
}

pub(crate) struct ContextGuard(pub(crate) *mut c_void);

impl Drop for ContextGuard {
  fn drop(&mut self) {
    if !self.0.is_null() {
      // Convert back to Box and let it drop properly
      unsafe {
        let _context = Box::from_raw(self.0.cast::<Context>());
        // Box destructor will clean up the Context
      }
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
