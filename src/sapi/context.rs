use crate::service::php::PhpRoute;
use anyhow::Error;
use bytes::Bytes;
use ext_php_rs::ffi::php_handle_auth_data;
use ext_php_rs::zend::SapiGlobals;
use headers::{ContentLength, ContentType, HeaderMapExt};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{HeaderMap, Request, Response, StatusCode, Version};
use std::ffi::{c_void, CString};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot::Sender;

pub(crate) struct Context {
  pub(crate) root: Arc<PathBuf>,
  route: PhpRoute,
  local_addr: SocketAddr,
  peer_addr: SocketAddr,
  request: Request<Bytes>,
  pub(crate) response_head: HeaderMap,
  pub(crate) buffer: Vec<u8>,
  respond_to: Option<Sender<Response<UnsyncBoxBody<Bytes, Error>>>>,
}

impl Context {
  pub(crate) fn new(
    root: Arc<PathBuf>,
    route: PhpRoute,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    request: Request<Bytes>,
    respond_to: Option<Sender<Response<UnsyncBoxBody<Bytes, Error>>>>,
  ) -> Self {
    Self {
      root,
      route,
      local_addr,
      peer_addr,
      request,
      response_head: HeaderMap::default(),
      buffer: Vec::default(),
      respond_to,
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
    self.local_addr
  }

  pub(crate) fn peer_addr(&self) -> SocketAddr {
    self.peer_addr
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

  pub(crate) fn send_response(
    &mut self,
    response: Response<UnsyncBoxBody<Bytes, Error>>,
  ) -> anyhow::Result<(), Response<UnsyncBoxBody<Bytes, Error>>> {
    self.respond_to.take().unwrap().send(response)
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
}
