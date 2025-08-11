use crate::Stream;
use crate::sapi::ext::SapiHeadersExt;
use crate::service::serve_php::PhpRoute;
use bytes::{Bytes, BytesMut};
use ext_php_rs::ffi::{php_handle_aborted_connection, php_handle_auth_data, php_output_end_all};
use ext_php_rs::zend::SapiGlobals;
use headers::{ContentLength, ContentType, HeaderMapExt};
use http_body_util::channel::Sender as BodyChannelSender;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Channel, Full};
use hyper::body::Frame;
use hyper::header::IntoHeaderName;
use hyper::http::HeaderValue;
use hyper::{HeaderMap, Method, Request, Response, StatusCode, Uri, Version};
use std::convert::Infallible;
use std::ffi::{CString, c_void};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot::{Receiver, Sender as OneShotSender};
use tracing::{error, instrument};

type ResponseBody = UnsyncBoxBody<Bytes, Infallible>;

#[derive(Debug)]
pub(crate) struct Context {
  root: Arc<PathBuf>,
  route: PhpRoute,
  stream: Arc<Stream>,
  request: Request<Bytes>,
  response_head: HeaderMap,
  buffer: BytesMut,
  sender: ContextSender,
  flushed: bool,
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
    Self {
      root,
      route,
      stream,
      request,
      sender,
      response_head: HeaderMap::default(),
      buffer: BytesMut::default(),
      flushed: false,
      request_finished: false,
    }
  }

  pub(crate) fn from_server_context(server_context: *mut c_void) -> Option<&'static mut Context> {
    unsafe { server_context.cast::<Self>().as_mut() }
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

  pub(crate) fn method(&self) -> &Method {
    self.request.method()
  }

  pub(crate) fn uri(&self) -> &Uri {
    self.request.uri()
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
    self.response_head.append(key, value);
  }

  pub(crate) fn buffer(&mut self) -> &mut BytesMut {
    &mut self.buffer
  }

  pub(crate) fn init_globals(&self) -> anyhow::Result<()> {
    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.request_info.request_method =
      CString::new(self.request.method().as_str())?.into_raw();

    sapi_globals.request_info.query_string = self
      .request
      .uri()
      .query()
      .and_then(|query| CString::new(query).ok())
      .map(|query| query.into_raw())
      .unwrap_or_else(std::ptr::null_mut);

    let path_translated = format!("{}{}", self.root.to_str().unwrap(), self.route.script_name());
    sapi_globals.request_info.path_translated = CString::new(path_translated)?.into_raw();

    sapi_globals.request_info.request_uri =
      CString::new(self.request.uri().to_string())?.into_raw();

    sapi_globals.request_info.content_length = self
      .request
      .headers()
      .typed_get::<ContentLength>()
      .map_or(0, |content_length| content_length.0.cast_signed());

    sapi_globals.request_info.content_type = self
      .request
      .headers()
      .typed_get::<ContentType>()
      .map_or(std::ptr::null_mut(), |content_type| {
        CString::new(content_type.to_string()).unwrap().into_raw()
      });

    if let Some(auth) = self.request.headers().get("Authorization") {
      unsafe {
        php_handle_auth_data(CString::new(auth.as_bytes())?.into_raw());
      }
    }

    Ok(())
  }

  #[instrument]
  pub(crate) fn flush(&mut self) {
    if !self.flushed {
      self.sender.send_head(self.response_head.clone());
    }

    self.flushed = true;
    self.sender.send_body(Bytes::from(self.buffer.clone()), false);
    self.buffer.clear();
  }

  pub(crate) fn is_request_finished(&self) -> bool {
    self.request_finished
  }

  #[instrument]
  pub(crate) fn finish_request(&mut self) -> bool {
    if self.request_finished {
      return false;
    }

    unsafe { php_output_end_all() }

    if self.flushed {
      self.sender.send_body(Bytes::from(self.buffer.clone()), true);
      self.request_finished = true;
      return true;
    }

    if self.sender.send_response(
      self.response_head.clone(),
      Full::new(Bytes::from(self.buffer.clone())).boxed_unsync(),
    ) {
      self.request_finished = true;
      return true;
    }

    false
  }
}

type ContextReceiver = (
  Receiver<Response<ResponseBody>>,
  Receiver<(StatusCode, HeaderMap)>,
  Channel<Bytes>,
  ContextSender,
);

#[derive(Default, Debug)]
pub(crate) struct ContextSender {
  head: Option<OneShotSender<(StatusCode, HeaderMap)>>,
  body: Option<BodyChannelSender<Bytes>>,
  response: Option<OneShotSender<Response<ResponseBody>>>,
}

impl ContextSender {
  pub(crate) fn receiver() -> ContextReceiver {
    let (head_tx, head_rx) = tokio::sync::oneshot::channel();
    let (body_tx, body_rx) = Channel::<Bytes>::new(1);
    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    (
      resp_rx,
      head_rx,
      body_rx,
      Self { head: Some(head_tx), body: Some(body_tx), response: Some(resp_tx) },
    )
  }

  #[instrument]
  pub(crate) fn send_response(&mut self, headers: HeaderMap, body: ResponseBody) -> bool {
    if let Some(resp_tx) = self.response.take() {
      unsafe {
        php_output_end_all();
      }

      let builder = Response::builder().status(SapiGlobals::get().sapi_headers().status());

      let mut response = builder.body(body).unwrap();
      *response.headers_mut() = headers;

      if resp_tx.send(response).is_ok() {
        return true;
      }
    }

    false
  }

  #[instrument]
  pub(crate) fn send_head(&mut self, headers: HeaderMap) {
    if let Some(head_tx) = self.head.take() {
      if head_tx.send((SapiGlobals::get().sapi_headers.status(), headers)).is_err() {
        unsafe { php_handle_aborted_connection() }
        error!("send head error");
      }
    }
  }

  #[instrument]
  pub(crate) fn send_body(&mut self, body: Bytes, finished: bool) {
    if let Some(mut body_tx) = self.body.take() {
      if body_tx.try_send(Frame::data(body)).is_err() {
        unsafe { php_handle_aborted_connection() }
        error!("send body error");
      }

      if !finished {
        self.body = Some(body_tx);
      }
    }
  }
}
