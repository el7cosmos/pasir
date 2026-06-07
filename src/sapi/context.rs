use std::ops::Sub;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use bytes::Bytes;
use ext_php_rs::embed::RequestInfo;
use ext_php_rs::embed::ServerVarRegistrar;
use ext_php_rs::zend::SapiGlobals;
use headers::Authorization;
use headers::ContentLength;
use headers::ContentType;
use headers::HeaderMapExt;
use headers::Host;
use headers::authorization::Basic;
use hyper::HeaderMap;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::Uri;
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
pub struct Context {
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

  pub(crate) fn script_name(&self) -> &str {
    &self.script_name
  }

  pub(crate) fn path_info(&self) -> Option<&str> {
    self.path_info.as_deref()
  }

  pub(crate) fn headers(&self) -> &HeaderMap {
    self.request.headers()
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

impl ext_php_rs::embed::ServerContext for Context {
  fn init_request_info(&self, info: &mut RequestInfo) {
    let uri = self.request.uri();
    let headers = self.request.headers();
    let path_translated = format!("{}{}", self.root.to_string_lossy(), self.script_name);

    info.request_method = Some(self.request.method().to_string());
    info.query_string = uri.query().map(|query| query.to_string());
    info.request_uri = Some(uri.to_string());
    info.path_translated = Some(path_translated);
    info.content_type = headers.typed_get::<ContentType>().map(|content_type| content_type.to_string());
    info.content_length = headers
      .typed_get::<ContentLength>()
      .map_or(0, |content_length| content_length.0.cast_signed());
    info.proto_num = match self.request.version() {
      Version::HTTP_09 => 900,
      Version::HTTP_10 => 1000,
      Version::HTTP_11 => 1100,
      Version::HTTP_2 => 2000,
      Version::HTTP_3 => 3000,
      _ => unreachable!(),
    };
    if let Some(auth) = headers.typed_get::<Authorization<Basic>>() {
      info.auth_user = Some(auth.username().to_string());
      info.auth_password = Some(auth.password().to_string());
    }
  }

  fn read_post(&mut self, buf: &mut [u8]) -> usize {
    let sapi_globals = SapiGlobals::get();

    let content_length = sapi_globals.request_info().content_length();
    if content_length == 0 {
      return 0;
    }

    // If we've read everything, return 0
    if sapi_globals.read_post_bytes >= content_length {
      return 0;
    }

    // Calculate how much we can read
    let to_read = buf.len().min(content_length.sub(sapi_globals.read_post_bytes) as usize);

    if to_read > self.request.body_mut().len() {
      return 0;
    }

    let bytes = self.request.body_mut().split_to(to_read);
    buf[..bytes.len()].copy_from_slice(&bytes);
    bytes.len()
  }

  fn read_cookies(&self) -> Option<&str> {
    self.headers().get("Cookie")?.to_str().ok()
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

  fn is_request_finished(&self) -> bool {
    self.request_finished
  }
}

impl ServerContext for Context {
  #[instrument(skip(self, registrar))]
  fn register_server_variables(&self, registrar: &mut ServerVarRegistrar) {
    registrar.register(
      "SERVER_SOFTWARE",
      &format!("{}/{} ({})", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_DESCRIPTION")),
    );

    registrar.register("REQUEST_URI", self.request.uri().path());
    registrar.register("REQUEST_METHOD", self.request.method().as_str());
    if let Some(query) = self.request.uri().query() {
      registrar.register("QUERY_STRING", query);
    }

    let root = self.root.to_str().unwrap_or_default();
    let path_info = self.path_info();
    let php_self = format!("{}{}", self.script_name, path_info.unwrap_or_default());

    registrar.register("PHP_SELF", &php_self);
    registrar.register("SERVER_PROTOCOL", &format!("{:?}", self.request.version()));
    registrar.register("DOCUMENT_ROOT", root);
    registrar.register("REMOTE_ADDR", &self.stream.peer_addr().ip().to_string());
    registrar.register("REMOTE_PORT", &self.stream.peer_addr().port().to_string());
    registrar.register("SCRIPT_FILENAME", &format!("{root}{}", self.script_name));
    registrar.register("SERVER_ADDR", &self.stream.local_addr().ip().to_string());
    registrar.register("SERVER_PORT", &self.stream.local_addr().port().to_string());
    registrar.register("SCRIPT_NAME", &self.script_name);
    if let Some(path_info) = path_info {
      registrar.register("PATH_INFO", path_info);
    }

    let headers = self.request.headers();
    if let Ok(uri) = match headers.typed_get::<Host>() {
      None => Uri::from_maybe_shared(""),
      Some(host) => Uri::from_str(host.hostname()),
    } {
      registrar.register("SERVER_NAME", uri.host().unwrap());
    }

    for (name, value) in headers.iter() {
      let header_name = format!("HTTP_{}", name.as_str().to_uppercase().replace('-', "_"));
      registrar.register(&header_name, value.to_str().unwrap_or_default());
    }
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
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
  use std::collections::HashMap;
  use std::net::Ipv4Addr;
  use std::path::PathBuf;
  use std::sync::Arc;

  use bytes::Bytes;
  use ext_php_rs::embed::RequestInfo;
  use ext_php_rs::embed::ServerContext as _;
  use ext_php_rs::embed::ServerVarRegistrar;
  use ext_php_rs::types::Zval;
  use ext_php_rs::zend::SapiGlobals;
  use hyper::Method;
  use hyper::Request;
  use hyper::Uri;
  use hyper::Version;
  use hyper::header::AUTHORIZATION;
  use hyper::header::CONTENT_LENGTH;
  use hyper::header::CONTENT_TYPE;
  use pasir_sapi::Sapi;
  use pasir_sapi::context::ServerContext;
  use pasir_sys::ZEND_RESULT_CODE_SUCCESS;

  use crate::sapi::context::Context;
  use crate::sapi::context::ContextBuilder;
  use crate::sapi::context::ContextSender;

  struct TestSapi;

  impl TestSapi {}

  impl ext_php_rs::embed::Sapi for TestSapi {
    type Context = Context;

    fn name() -> &'static str {
      ""
    }

    fn pretty_name() -> &'static str {
      ""
    }

    fn ub_write(_ctx: &mut Self::Context, buf: &[u8]) -> usize {
      buf.len()
    }

    fn log_message(_message: &str, _syslog_type: i32) {}
  }

  impl Sapi for TestSapi {}

  #[rstest::rstest]
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
  fn test_flush() {
    let sapi = TestSapi::build_module().unwrap().into_raw();
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() };
    unsafe { pasir_sys::sapi_startup(sapi) };
    unsafe { pasir_sys::php_module_startup(sapi, std::ptr::null_mut()) };

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

    unsafe { pasir_sys::php_module_shutdown() };
    unsafe { pasir_sys::sapi_shutdown() };
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() };
  }

  #[test]
  fn test_init_request_info() {
    let uri = Uri::builder().path_and_query("/foo?bar=baz").build().unwrap();
    let request = Request::builder()
      .method(Method::POST)
      .version(Version::HTTP_3)
      .header(CONTENT_LENGTH, "Foo Bar".len())
      .header(CONTENT_TYPE, "text/plain")
      .header(AUTHORIZATION, "Basic Zm9vOmJhcg==")
      .uri(uri)
      .body(Bytes::default())
      .unwrap();
    let mut context = ContextBuilder::default().request(request).build();
    context.script_name = "./index.php".to_string();

    let mut request_info = RequestInfo::default();
    context.init_request_info(&mut request_info);

    assert_eq!(request_info.request_method, Some(Method::POST.to_string()));
    assert_eq!(request_info.query_string, Some("bar=baz".to_string()));
    assert_eq!(request_info.request_uri, Some("/foo?bar=baz".to_string()));
    assert_eq!(request_info.path_translated, Some("./index.php".to_string()));
    assert_eq!(request_info.content_type, Some("text/plain".to_string()));
    assert_eq!(request_info.content_length, "Foo Bar".len() as i64);
    assert_eq!(request_info.proto_num, 3000);
    assert_eq!(request_info.auth_user, Some("foo".to_string()));
    assert_eq!(request_info.auth_password, Some("bar".to_string()));
  }

  #[test]
  fn test_read_post() {
    let sapi = TestSapi::build_module().unwrap().into_raw();
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() };
    unsafe { pasir_sys::sapi_startup(sapi) };
    unsafe { pasir_sys::php_module_startup(sapi, std::ptr::null_mut()) };

    let request = Request::new(Bytes::from_static(b"Foo"));
    let mut context = ContextBuilder::default().request(request).build();

    SapiGlobals::get_mut().request_info.content_length = 3;

    let buf: &mut [u8; 1] = &mut Default::default();
    assert_eq!(context.read_post(buf), 1);
    assert_eq!(str::from_utf8(buf), Ok("F"));

    let buf: &mut [u8; 2] = &mut Default::default();
    assert_eq!(context.read_post(buf), 2);
    assert_eq!(str::from_utf8(buf), Ok("oo"));

    SapiGlobals::get_mut().read_post_bytes = 3;
    let buf = &mut [0u8; 3];
    buf.copy_from_slice(b"Bar");
    assert_eq!(context.read_post(buf), 0);
    assert_eq!(str::from_utf8(buf), Ok("Bar"));

    unsafe { pasir_sys::php_module_shutdown() };
    unsafe { pasir_sys::sapi_shutdown() };
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() };
  }

  #[test]
  fn test_register_server_variables() {
    let sapi = TestSapi::build_module().unwrap().into_raw();
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() };
    unsafe { pasir_sys::sapi_startup(sapi) };
    unsafe { pasir_sys::php_module_startup(sapi, std::ptr::null_mut()) };

    let localhost = Ipv4Addr::LOCALHOST;
    let root = PathBuf::from("/foo");
    let request = Request::builder()
      .header("Cookie", "foo=bar")
      .header("Host", localhost.to_string())
      .uri(Uri::builder().path_and_query("/foo/bar?foo=bar").build().unwrap())
      .body(Bytes::default())
      .unwrap();
    let context = ContextBuilder::default()
      .root(root)
      .script_name("/index.php")
      .path_info("/foo/bar")
      .request(request)
      .build();

    assert_eq!(unsafe { pasir_sys::php_request_startup() }, ZEND_RESULT_CODE_SUCCESS);
    unsafe { pasir_sys::php_request_shutdown(std::ptr::null_mut()) };

    let mut vars = Zval::new();
    let _ = vars.set_array(HashMap::<String, String>::new());
    assert!(vars.is_array());
    let vars_raw = Box::into_raw(Box::new(vars));
    context.register_server_variables(&mut unsafe { ServerVarRegistrar::from_raw(vars_raw) });

    let zval = unsafe { Box::from_raw(vars_raw) };
    let vars = zval.array().unwrap();
    assert!(vars.get("SERVER_SOFTWARE").is_some());
    assert_eq!(vars.get("REQUEST_URI").map(|var| var.str()), Some(Some("/foo/bar")));
    assert_eq!(vars.get("REQUEST_METHOD").map(|var| var.str()), Some(Some("GET")));
    assert_eq!(vars.get("QUERY_STRING").map(|var| var.str()), Some(Some("foo=bar")));
    assert_eq!(vars.get("PHP_SELF").map(|var| var.str()), Some(Some("/index.php/foo/bar")));
    assert_eq!(vars.get("SERVER_PROTOCOL").map(|var| var.str()), Some(Some("HTTP/1.1")));
    assert_eq!(vars.get("DOCUMENT_ROOT").map(|var| var.str()), Some(Some("/foo")));
    assert_eq!(vars.get("REMOTE_ADDR").map(|var| var.string()), Some(Some(localhost.to_string())));
    assert_eq!(vars.get("REMOTE_PORT").map(|var| var.str()), Some(Some("0")));
    assert_eq!(vars.get("SCRIPT_FILENAME").map(|var| var.str()), Some(Some("/foo/index.php")));
    assert_eq!(vars.get("SERVER_ADDR").map(|var| var.string()), Some(Some(localhost.to_string())));
    assert_eq!(vars.get("SERVER_PORT").map(|var| var.str()), Some(Some("0")));
    assert_eq!(vars.get("SCRIPT_NAME").map(|var| var.str()), Some(Some("/index.php")));
    assert_eq!(vars.get("PATH_INFO").map(|var| var.str()), Some(Some("/foo/bar")));
    assert_eq!(vars.get("SERVER_NAME").map(|var| var.string()), Some(Some(localhost.to_string())));
    assert_eq!(vars.get("HTTP_COOKIE").map(|var| var.str()), Some(Some("foo=bar")));
    assert_eq!(vars.get("HTTP_HOST").map(|var| var.string()), Some(Some(localhost.to_string())));

    unsafe { pasir_sys::php_module_shutdown() };
    unsafe { pasir_sys::sapi_shutdown() };
  }
}
