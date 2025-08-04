use crate::Stream;
use crate::service::serve_php::PhpRoute;
use bytes::{Bytes, BytesMut};
use ext_php_rs::ffi::{php_handle_auth_data, php_output_end_all};
use ext_php_rs::zend::SapiGlobals;
use headers::{ContentLength, ContentType, HeaderMapExt};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{HeaderMap, Request, Response, StatusCode, Version};
use std::convert::Infallible;
use std::ffi::{CString, c_void};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot::Sender;

pub(crate) struct Context {
  pub(crate) root: Arc<PathBuf>,
  route: PhpRoute,
  stream: Stream,
  request: Request<Bytes>,
  pub(crate) response_head: HeaderMap,
  buffer: BytesMut,
  respond_to: Option<Sender<Response<UnsyncBoxBody<Bytes, Infallible>>>>,
  request_finished: bool,
}

impl Context {
  pub(crate) fn new(
    root: Arc<PathBuf>,
    route: PhpRoute,
    stream: Stream,
    request: Request<Bytes>,
    respond_to: Option<Sender<Response<UnsyncBoxBody<Bytes, Infallible>>>>,
  ) -> Self {
    Self {
      root,
      route,
      stream,
      request,
      response_head: HeaderMap::default(),
      buffer: BytesMut::default(),
      respond_to,
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

  pub(crate) fn headers(&self) -> &HeaderMap {
    self.request.headers()
  }

  pub(crate) fn version(&self) -> Version {
    self.request.version()
  }

  pub(crate) fn body_mut(&mut self) -> &mut Bytes {
    self.request.body_mut()
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

  pub(crate) fn is_request_finished(&self) -> bool {
    self.request_finished
  }

  pub(crate) fn finish_request(&mut self) -> bool {
    if self.request_finished {
      return false;
    }

    if let Some(sender) = self.respond_to.take() {
      unsafe {
        php_output_end_all();
      }

      let rc = SapiGlobals::get().sapi_headers().http_response_code;
      let builder = Response::builder().status(match rc.is_positive() {
        true => StatusCode::from_u16(rc.cast_unsigned() as u16).unwrap_or_default(),
        false => StatusCode::default(),
      });

      let mut response =
        builder.body(Full::new(Bytes::from(self.buffer.clone())).boxed_unsync()).unwrap();
      *response.headers_mut() = self.response_head.clone();

      if sender.send(response).is_ok() {
        self.request_finished = true;
        return true;
      }
    }

    false
  }
}
