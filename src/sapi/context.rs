use anyhow::Error;
use bytes::Bytes;
use ext_php_rs::ffi::php_handle_auth_data;
use ext_php_rs::zend::SapiGlobals;
use headers::{ContentLength, ContentType, HeaderMapExt};
use http_body_util::combinators::UnsyncBoxBody;
use hyper::http::request::Parts;
use hyper::{HeaderMap, Response, Version};
use std::ffi::{c_void, CString};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot::Sender;

pub(crate) struct Context {
  pub(crate) root: Arc<PathBuf>,
  script_name: String,
  path_info: Option<String>,
  local_addr: SocketAddr,
  peer_addr: SocketAddr,
  head: Parts,
  body: Bytes,
  pub(crate) response_head: HeaderMap,
  pub(crate) buffer: Vec<u8>,
  respond_to: Option<Sender<Response<UnsyncBoxBody<Bytes, Error>>>>,
}

impl Context {
  pub(crate) fn new(
    root: Arc<PathBuf>,
    script_name: String,
    path_info: Option<String>,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    head: Parts,
    body: Bytes,
    respond_to: Option<Sender<Response<UnsyncBoxBody<Bytes, Error>>>>,
  ) -> Self {
    Self {
      root,
      script_name,
      path_info,
      local_addr,
      peer_addr,
      head,
      body,
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

  pub(crate) fn script_name(&self) -> &str {
    &self.script_name
  }

  pub(crate) fn path_info(&self) -> Option<&str> {
    self.path_info.as_deref()
  }

  pub(crate) fn local_addr(&self) -> SocketAddr {
    self.local_addr
  }

  pub(crate) fn peer_addr(&self) -> SocketAddr {
    self.peer_addr
  }

  pub(crate) fn headers(&self) -> &HeaderMap {
    &self.head.headers
  }

  pub(crate) fn version(&self) -> Version {
    self.head.version
  }

  pub(crate) fn body_mut(&mut self) -> &mut Bytes {
    &mut self.body
  }

  pub(crate) fn send_response(
    &mut self,
    response: Response<UnsyncBoxBody<Bytes, Error>>,
  ) -> Result<(), Response<UnsyncBoxBody<Bytes, Error>>> {
    self.respond_to.take().unwrap().send(response)
    // self.respond_to.send(response)
  }

  pub(crate) fn init_globals(&self) -> anyhow::Result<()> {
    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.request_info.request_method = CString::new(self.head.method.as_str())?.into_raw();

    sapi_globals.request_info.query_string = self
      .head
      .uri
      .query()
      .and_then(|query| CString::new(query).ok())
      .map(|query| query.into_raw())
      .unwrap_or_else(std::ptr::null_mut);

    let path_translated = format!("{}{}", self.root.to_str().unwrap(), self.script_name);
    sapi_globals.request_info.path_translated = CString::new(path_translated)?.into_raw();

    sapi_globals.request_info.request_uri = CString::new(self.head.uri.to_string())?.into_raw();

    sapi_globals.request_info.content_length = self
      .head
      .headers
      .typed_get::<ContentLength>()
      .map_or(0, |content_length| content_length.0.cast_signed());

    sapi_globals.request_info.content_type =
      self.head.headers.typed_get::<ContentType>().map_or(std::ptr::null_mut(), |content_type| {
        CString::new(content_type.to_string()).unwrap().into_raw()
      });

    if let Some(auth) = self.head.headers.get("Authorization") {
      unsafe {
        php_handle_auth_data(CString::new(auth.as_bytes())?.into_raw());
      }
    }

    Ok(())
  }
}
