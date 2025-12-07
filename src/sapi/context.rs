use std::ffi::CString;
use std::ffi::NulError;
use std::ffi::c_char;
use std::ffi::c_int;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use ext_php_rs::zend::SapiGlobals;
use headers::ContentLength;
use headers::ContentType;
use headers::HeaderMapExt;
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
use pasir_sapi::context::ServerContext;
use tokio::sync::oneshot::Receiver;
use tokio::sync::oneshot::Sender as OneShotSender;
use tracing::debug;
use tracing::instrument;

use crate::cli::serve::Stream;
use crate::sapi::ext::FromSapiHeaders;

#[derive(Clone, Debug, Default)]
pub(crate) enum ResponseType {
  #[default]
  Full,
  Chunked,
}

#[derive(Debug, Default)]
pub(crate) struct Context {
  root: Arc<PathBuf>,
  script_name: String,
  path_info: Option<String>,
  stream: Arc<Stream>,
  request: Request<Bytes>,
  headers: HeaderMap,
  sender: ContextSender,
  request_finished: bool,
}

impl Context {
  pub(crate) fn new(root: Arc<PathBuf>, stream: Arc<Stream>, request: Request<Bytes>, sender: ContextSender) -> Self {
    let uri = request.uri().path().to_string();
    let mut context = Self {
      root,
      script_name: Default::default(),
      path_info: None,
      stream,
      request,
      sender,
      headers: Default::default(),
      request_finished: false,
    };
    context.parse_uri(uri, None);
    context
  }

  fn parse_uri(&mut self, uri: String, path_info: Option<String>) {
    let root = self.root.as_path();
    // Normalize the URI by removing trailing slashes before processing
    let normalized_uri = uri.trim_end_matches('/');
    let file = root.join(normalized_uri.trim_start_matches('/'));
    self.path_info = path_info;

    // If we are at the document root, route to `/index.php`.
    if file == root {
      self.script_name = "/index.php".to_string();
      return;
    }

    if file.is_file() && normalized_uri.ends_with(".php") {
      self.script_name = normalized_uri.to_string();
      return;
    }

    if file.is_dir() {
      let index = file.join("index.php");
      if index.is_file() {
        self.script_name = format!("{}/index.php", normalized_uri);
        return;
      }
    }

    if let Some(name) = file.file_name() {
      let suffix = format!("/{}", name.to_string_lossy());
      let path_info = format!("{}{}", suffix, self.path_info.take().unwrap_or_default());
      if let Some(parent_uri) = normalized_uri.strip_suffix(suffix.as_str()) {
        self.parse_uri(parent_uri.to_string(), Some(path_info));
      }
    }
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

  pub(crate) fn append_response_header<K>(&mut self, key: K, value: HeaderValue)
  where
    K: IntoHeaderName,
  {
    self.headers.append(key, value);
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
      let (mut parts, _) = Response::<Bytes>::default().into_parts();
      parts.headers = std::mem::take(&mut self.headers);
      parts.extensions.insert(ResponseType::Chunked);
      return self.sender.send_head(parts);
    }

    false
  }
}

impl ServerContext for Context {
  #[instrument(skip(self), err)]
  fn init_sapi_globals(&mut self) -> Result<(), NulError> {
    let uri = self.request.uri();
    let headers = self.request.headers();
    let path_translated = format!("{}{}", self.root.to_string_lossy(), self.script_name);

    let mut sapi_globals = SapiGlobals::get_mut();

    sapi_globals.sapi_headers.http_response_code = StatusCode::OK.as_u16() as c_int;
    sapi_globals.request_info.request_method = CString::new(self.request.method().as_str())?.into_raw().cast_const();
    sapi_globals.request_info.query_string = uri
      .query()
      .and_then(|query| CString::new(query).ok())
      .map(|query| query.into_raw())
      .unwrap_or_else(std::ptr::null_mut);
    sapi_globals.request_info.path_translated = CString::new(path_translated)?.into_raw();
    sapi_globals.request_info.request_uri = CString::new(uri.to_string())?.into_raw();
    sapi_globals.request_info.content_length = headers
      .typed_get::<ContentLength>()
      .map_or(0, |content_length| content_length.0.cast_signed());
    sapi_globals.request_info.content_type = headers.typed_get::<ContentType>().map_or(Ok(std::ptr::null()), |content_type| {
      CString::new(content_type.to_string()).map(|content_type| content_type.into_raw().cast_const())
    })?;

    if let Some(auth) = headers.get("Authorization") {
      unsafe {
        pasir_sys::php_handle_auth_data(CString::new(auth.as_bytes())?.as_ptr());
      }
    }

    let proto_num = match self.request.version() {
      Version::HTTP_09 => 900,
      Version::HTTP_10 => 1000,
      Version::HTTP_11 => 1100,
      Version::HTTP_2 => 2000,
      Version::HTTP_3 => 3000,
      _ => unreachable!(),
    };
    sapi_globals.request_info.proto_num = proto_num;

    Ok(())
  }

  fn read_post(&mut self, buffer: *mut c_char, to_read: usize) -> usize {
    if to_read > self.request.body_mut().len() {
      return 0;
    }

    let bytes = self.request.body_mut().split_to(to_read);
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), buffer, bytes.len()) };
    bytes.len()
  }

  fn is_request_finished(&self) -> bool {
    self.request_finished
  }

  #[instrument(skip(self))]
  fn finish_request(&mut self) -> bool {
    if self.request_finished {
      return false;
    }

    unsafe { pasir_sys::php_output_end_all() }

    if let Some(body_tx) = self.sender.body.take() {
      body_tx.abort();
    }

    self.request_finished = true;

    if self.sender.head.is_some() {
      let (mut parts, _) = Response::<Bytes>::default().into_parts();
      parts.headers = std::mem::take(&mut self.headers);
      return self.sender.send_head(parts);
    }

    true
  }
}

#[cfg(test)]
#[derive(Default)]
pub struct ContextBuilder(Context);

#[cfg(test)]
impl ContextBuilder {
  pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
    self.0.root = Arc::new(root.into());
    self
  }

  pub fn script_name(mut self, script_name: impl Into<String>) -> Self {
    self.0.script_name = script_name.into();
    self
  }

  pub fn path_info(mut self, path_info: impl Into<String>) -> Self {
    self.0.path_info = Some(path_info.into());
    self
  }

  pub fn request(mut self, request: Request<Bytes>) -> Self {
    self.0.request = request;
    self
  }

  pub fn sender(mut self, sender: ContextSender) -> Self {
    self.0.sender = sender;
    self
  }

  pub fn build(self) -> Context {
    self.0
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
    let sender = Self {
      head: Some(head_tx),
      body: Some(body_tx),
    };
    (head_rx, body_rx, sender)
  }

  #[instrument(skip(self))]
  pub(crate) fn send_head(&mut self, mut headers: Parts) -> bool {
    if let Some(head_tx) = self.head.take() {
      if let Ok(status) = StatusCode::from_sapi_headers(SapiGlobals::get().sapi_headers()) {
        headers.status = status;
      }
      if head_tx.send(headers).is_err() {
        pasir_sapi::util::handle_abort_connection();
        return false;
      }
    }

    true
  }
}

#[cfg(test)]
mod tests {
  use std::ffi::CString;
  use std::ffi::c_int;
  use std::path::PathBuf;
  use std::sync::Arc;

  use bytes::Bytes;
  use ext_php_rs::zend::SapiGlobals;
  use hyper::Method;
  use hyper::Request;
  use hyper::StatusCode;
  use hyper::Uri;
  use hyper::Version;
  use hyper::header::CONTENT_LENGTH;
  use hyper::header::CONTENT_TYPE;
  use pasir_sapi::context::ServerContext;
  use rstest::rstest;

  use crate::sapi::context::Context;
  use crate::sapi::context::ContextBuilder;
  use crate::sapi::context::ContextSender;
  use crate::sapi::tests::TestSapi;

  #[rstest]
  #[case("/", "/index.php", None)]
  #[case("/..", "", None)]
  #[case("/foo", "/foo/index.php", None)]
  #[case("/foo/", "/foo/index.php", None)]
  #[case("/foo/bar/baz", "/foo/index.php", Some("/bar/baz"))]
  #[case("/foo/index.php/bar", "/foo/index.php", Some("/bar"))]
  #[case("/foo/foo.php", "/foo/foo.php", None)]
  #[case("/foo/foo.php/bar", "/foo/foo.php", Some("/bar"))]
  #[case("/bar/baz", "/index.php", Some("/bar/baz"))]
  #[trace]
  fn test_parse_uri(#[case] request_uri: String, #[case] script_name: &str, #[case] path_info: Option<&str>) {
    let root = PathBuf::from("tests/fixtures/root");
    let uri = Uri::builder().path_and_query(request_uri).build().unwrap();
    let request = Request::builder().uri(uri).body(Bytes::default()).unwrap();

    let context = Context::new(Arc::new(root), Default::default(), request, Default::default());
    assert_eq!(context.script_name(), script_name);
    assert_eq!(context.path_info(), path_info);
  }

  #[test]
  fn test_init_sapi_globals() {
    let _guard = TestSapi::new();

    let uri = Uri::builder().path_and_query("/foo?bar=baz").build().unwrap();
    let request = Request::builder()
      .method(Method::POST)
      .version(Version::HTTP_3)
      .header(CONTENT_LENGTH, "Foo Bar".len())
      .header(CONTENT_TYPE, "text/plain")
      .uri(uri)
      .body(Bytes::default())
      .unwrap();
    let mut context = ContextBuilder::default().request(request).build();
    context.script_name = "./index.php".to_string();
    let results = context.init_sapi_globals();
    assert!(results.is_ok());

    let sapi_globals = SapiGlobals::get();
    assert_eq!(sapi_globals.sapi_headers().http_response_code, StatusCode::OK.as_u16() as c_int);

    let request_info = sapi_globals.request_info();
    assert_eq!(request_info.request_method(), Some(Method::POST.as_str()));
    assert_eq!(request_info.query_string(), Some("bar=baz"));
    assert_eq!(request_info.path_translated(), Some("./index.php"));
    assert_eq!(request_info.request_uri(), Some("/foo?bar=baz"));
    assert_eq!(request_info.content_length(), "Foo Bar".len() as i64);
    assert_eq!(request_info.content_type(), Some("text/plain"));
    assert_eq!(request_info.auth_user(), None);
    assert_eq!(request_info.proto_num(), 3000);
  }

  #[test]
  fn test_flush() {
    let _sapi = TestSapi::new();

    let (_head_rx, _body_rx, context_sender) = ContextSender::receiver();
    let context = ContextBuilder::default().sender(context_sender).build();
    SapiGlobals::get_mut().server_context = context.into_raw().cast();

    unsafe { pasir_sys::php_output_startup() };
    let mut context = unsafe { Context::from_raw(SapiGlobals::get().server_context) };

    // assert that `flush` is true if the request not finished yet.
    assert!(context.flush());
    // assert that `finish_request` is true if the request not finished yet.
    assert!(context.finish_request());
    // assert that `flush` is false if the request finished already.
    assert!(!context.flush());
  }

  #[test]
  fn test_read_post() {
    let _sapi = TestSapi::new();

    let request = Request::new(Bytes::from_static(b"Foo"));
    let mut context = ContextBuilder::default().request(request).build();

    let buffer_raw = CString::default().into_raw();
    assert_eq!(context.read_post(buffer_raw, 1), 1);
    let buffer = unsafe { CString::from_raw(buffer_raw) };
    assert_eq!(buffer.as_c_str(), c"F");

    let buffer_raw = CString::default().into_raw();
    assert_eq!(context.read_post(buffer_raw, 2), 2);
    // SapiGlobals::get_mut().read_post_bytes = 3;
    let buffer = unsafe { CString::from_raw(buffer_raw) };
    assert_eq!(buffer.as_c_str(), c"oo");

    let buffer_raw = CString::default().into_raw();
    assert_eq!(context.read_post(buffer_raw, 3), 0);
    let buffer = unsafe { CString::from_raw(buffer_raw) };
    assert_eq!(buffer.as_c_str(), c"");
  }
}
