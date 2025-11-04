pub(crate) mod context;
mod ext;
mod util;
mod variables;

use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::ffi::c_int;
use std::ffi::c_void;
use std::ops::Sub;
use std::str::FromStr;
use std::time::SystemTime;

use bytes::Bytes;
use bytes::BytesMut;
use ext_php_rs::builders::SapiBuilder;
use ext_php_rs::prelude::*;
use ext_php_rs::types::Zval;
use ext_php_rs::zend::FunctionEntry;
use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::SapiHeader;
use ext_php_rs::zend::SapiModule;
use headers::HeaderMapExt;
use headers::Host;
use hyper::Uri;
use hyper::header::HeaderName;
use hyper::header::HeaderValue;
use pasir_sys::ZEND_RESULT_CODE;
use pasir_sys::ZEND_RESULT_CODE_FAILURE;
use pasir_sys::ZEND_RESULT_CODE_SUCCESS;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::instrument;
use tracing::trace;
use tracing::warn;

use crate::free_raw_cstring;
use crate::free_raw_cstring_mut;
use crate::sapi::context::Context;
use crate::sapi::util::*;
use crate::sapi::variables::*;

#[derive(Clone, Debug)]
pub(crate) struct Sapi(pub(crate) *mut SapiModule);

unsafe impl Send for Sapi {}
unsafe impl Sync for Sapi {}

impl Sapi {
  pub(crate) fn new(php_info_as_text: bool, ini_entries: Option<String>) -> Self {
    let mut builder = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
      .startup_function(startup)
      .shutdown_function(shutdown)
      .deactivate_function(deactivate)
      .ub_write_function(ub_write)
      .flush_function(flush)
      .send_header_function(send_header)
      .read_post_function(read_post)
      .read_cookies_function(read_cookies)
      .register_server_variables_function(register_server_variables)
      .log_message_function(log_message)
      .get_request_time_function(get_request_time);

    if let Some(entries) = ini_entries {
      builder = builder.ini_entries(entries);
    }

    let function_entry =
      wrap_function!(pasir_finish_request).build().expect("Failed to build functions");
    let mut function_alias = function_entry;
    function_alias.fname =
      CString::new("fastcgi_finish_request").expect("String contain nul byte").into_raw();

    let functions = vec![function_entry, function_alias, FunctionEntry::end()];

    let mut sapi_module = builder.build().unwrap();
    sapi_module.sapi_error = Some(pasir_sys::zend_error);
    sapi_module.phpinfo_as_text = php_info_as_text as c_int;
    sapi_module.additional_functions = Box::into_raw(functions.into_boxed_slice()).cast();
    Self(sapi_module.into_raw())
  }

  pub(crate) fn startup(&self) -> Result<(), ()> {
    let sapi_module_ptr = self.0;
    let sapi_module = unsafe { *sapi_module_ptr };

    let ini_entries = match sapi_module.ini_entries.is_null() {
      true => None,
      false => Some(sapi_module.ini_entries),
    };
    unsafe { pasir_sys::sapi_startup(sapi_module_ptr) };
    if let Some(entries) = ini_entries {
      unsafe { (*sapi_module_ptr).ini_entries = entries }
    }

    let startup = sapi_module.startup.expect("startup function is null");
    match unsafe { startup(sapi_module_ptr) } {
      ZEND_RESULT_CODE_SUCCESS => Ok(()),
      ZEND_RESULT_CODE_FAILURE => Err(()),
      _ => Err(()),
    }
  }

  pub(crate) fn shutdown(&self) {
    if let Some(shutdown) = unsafe { *self.0 }.shutdown {
      unsafe { shutdown(self.0) };
    }
    unsafe { pasir_sys::sapi_shutdown() }
  }
}

impl Drop for Sapi {
  fn drop(&mut self) {
    unsafe {
      let sapi_module = Box::from_raw(self.0);

      drop(CString::from_raw(sapi_module.name));
      drop(CString::from_raw(sapi_module.pretty_name));
      if !sapi_module.ini_entries.is_null() {
        #[cfg(not(php83))]
        drop(CString::from_raw(sapi_module.ini_entries));
        #[cfg(php83)]
        drop(CString::from_raw(sapi_module.ini_entries.cast_mut()));
      }

      let additional_functions = std::slice::from_raw_parts(sapi_module.additional_functions, 3);
      if let Some(function) = additional_functions.first() {
        drop(CString::from_raw(function.fname.cast_mut()));
        drop(Box::from_raw(function.arg_info.cast_mut()));
      }
      if let Some(function) = additional_functions.get(1) {
        drop(CString::from_raw(function.fname.cast_mut()));
      }
      drop(Box::from_raw(sapi_module.additional_functions.cast_mut()));
    };
  }
}

extern "C" fn startup(sapi: *mut SapiModule) -> ZEND_RESULT_CODE {
  unsafe { pasir_sys::php_module_startup(sapi, std::ptr::null_mut()) }
}

extern "C" fn shutdown(_sapi: *mut SapiModule) -> ZEND_RESULT_CODE {
  unsafe { pasir_sys::php_module_shutdown() };
  ZEND_RESULT_CODE_SUCCESS
}

extern "C" fn deactivate() -> ZEND_RESULT_CODE {
  let sapi_globals = SapiGlobals::get();
  if !sapi_globals.sapi_started {
    return ZEND_RESULT_CODE_SUCCESS;
  }

  if sapi_globals.server_context.is_null() {
    return ZEND_RESULT_CODE_SUCCESS;
  }

  let mut request_info = sapi_globals.request_info;
  free_raw_cstring!(request_info, request_method);
  free_raw_cstring_mut!(request_info, query_string);
  free_raw_cstring_mut!(request_info, request_uri);
  free_raw_cstring!(request_info, content_type);
  free_raw_cstring_mut!(request_info, cookie_data);

  let mut context = unsafe { Context::from_raw(sapi_globals.server_context) };
  drop(sapi_globals);
  if !context.is_request_finished() && !context.finish_request() {
    trace!("finish request failed");
    handle_abort_connection();
  }
  SapiGlobals::get_mut().server_context = std::ptr::null_mut();

  ZEND_RESULT_CODE_SUCCESS
}

extern "C" fn ub_write(str: *const c_char, str_length: usize) -> usize {
  if str.is_null() || str_length == 0 {
    return 0;
  }

  // Not in a server context, write to stdout.
  if SapiGlobals::get().server_context.is_null() {
    let mut bytes = unsafe { std::slice::from_raw_parts(str.cast(), str_length) }.to_vec();
    if bytes.last() != Some(&0) {
      bytes.push(0);
    }

    return match CStr::from_bytes_with_nul(&bytes) {
      Ok(s) => {
        print!("{}", s.to_string_lossy());
        str_length
      }
      Err(_) => 0,
    };
  }

  let context = Context::from_server_context(SapiGlobals::get().server_context);
  if context.is_request_finished() {
    return 0;
  }

  let char = unsafe { std::slice::from_raw_parts(str.cast(), str_length) };
  match context.ub_write(Bytes::from(BytesMut::from(char))) {
    true => str_length,
    false => {
      handle_abort_connection();
      0
    }
  }
}

extern "C" fn flush(server_context: *mut c_void) {
  if !server_context.is_null() {
    Context::from_server_context(server_context).flush();
  }
}

extern "C" fn send_header(header: *mut SapiHeader, server_context: *mut c_void) {
  if server_context.is_null() {
    return;
  }

  if let Some(sapi_header) = unsafe { header.as_ref() } {
    if sapi_header.header.is_null() {
      return;
    }

    let context = Context::from_server_context(server_context);
    if let Some(value) = sapi_header.value() {
      context.append_response_header(
        HeaderName::from_str(sapi_header.name()).unwrap(),
        HeaderValue::from_str(value).unwrap(),
      );
    }
  }
}

extern "C" fn read_post(buffer: *mut c_char, length: usize) -> usize {
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
  let to_read = length.min(content_length.sub(sapi_globals.read_post_bytes) as usize);

  let context = Context::from_server_context(sapi_globals.server_context);
  let bytes = context.body_mut().split_to(to_read);
  unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), buffer, bytes.len()) };
  bytes.len()
}

extern "C" fn read_cookies() -> *mut c_char {
  Context::from_server_context(SapiGlobals::get().server_context)
    .headers()
    .get("Cookie")
    .map(|cookie| CString::new(cookie.to_str().unwrap()).unwrap().into_raw())
    .unwrap_or(std::ptr::null_mut())
}

extern "C" fn register_server_variables(vars: *mut Zval) {
  register_variable(
    SERVER_SOFTWARE,
    format!(
      "{}/{} ({})",
      env!("CARGO_PKG_NAME"),
      env!("CARGO_PKG_VERSION"),
      env!("CARGO_PKG_DESCRIPTION"),
    ),
    vars,
  );

  let sapi_globals = SapiGlobals::get();
  let request_info = sapi_globals.request_info();
  if !request_info.request_uri.is_null() {
    unsafe {
      pasir_sys::php_register_variable(
        REQUEST_URI.as_ptr(),
        request_info.request_uri.cast_const(),
        vars,
      );
    }
  }
  if !request_info.request_method.is_null() {
    unsafe {
      pasir_sys::php_register_variable(REQUEST_METHOD.as_ptr(), request_info.request_method, vars);
    }
  }
  if !request_info.query_string.is_null() {
    unsafe {
      pasir_sys::php_register_variable(
        QUERY_STRING.as_ptr(),
        request_info.query_string.cast_const(),
        vars,
      );
    }
  }

  if sapi_globals.server_context.is_null() {
    return;
  }

  let context = Context::from_server_context(sapi_globals.server_context);
  let root = context.root().to_str().unwrap_or_default();
  let script_name = context.script_name();
  let path_info = context.path_info();
  let php_self = format!("{}{}", script_name, path_info.unwrap_or_default());

  register_variable(PHP_SELF, php_self, vars);
  register_variable(SERVER_PROTOCOL, format!("{:?}", context.version()), vars);
  register_variable(DOCUMENT_ROOT, root, vars);
  register_variable(REMOTE_ADDR, context.peer_addr().ip().to_string(), vars);
  register_variable(REMOTE_PORT, context.peer_addr().port().to_string(), vars);
  register_variable(SCRIPT_FILENAME, format!("{root}{script_name}"), vars);
  register_variable(SERVER_ADDR, context.local_addr().ip().to_string(), vars);
  register_variable(SERVER_PORT, context.local_addr().port().to_string(), vars);
  register_variable(SCRIPT_NAME, script_name, vars);
  if let Some(path_info) = path_info {
    register_variable(PATH_INFO, path_info, vars);
  }

  let headers = context.headers();

  if let Ok(uri) = match headers.typed_get::<Host>() {
    None => Uri::from_maybe_shared(""),
    Some(host) => Uri::from_str(host.hostname()),
  } {
    register_variable(SERVER_NAME, uri.host().unwrap(), vars);
  }

  for (name, value) in headers.iter() {
    let header_name = format!("HTTP_{}", name.as_str().to_uppercase().replace('-', "_"));
    register_variable(CString::new(header_name).unwrap().as_c_str(), value.to_str().unwrap(), vars);
  }
}

extern "C" fn log_message(message: *const c_char, syslog_type_int: c_int) {
  unsafe {
    let error_message = CStr::from_ptr(message).to_string_lossy();
    match syslog_type_int {
      0..=3 => error!("{error_message}"),
      4 => warn!("{error_message}"),
      5 | 6 => info!("{error_message}"),
      7 => debug!("{error_message}"),
      _ => (),
    };
  }
}

#[instrument(skip(time))]
extern "C" fn get_request_time(time: *mut f64) -> c_int {
  match SystemTime::UNIX_EPOCH.elapsed() {
    Ok(timestamp) => {
      unsafe { time.write(timestamp.as_secs_f64()) };
      ZEND_RESULT_CODE_SUCCESS
    }
    Err(e) => {
      error!("{e}");
      ZEND_RESULT_CODE_FAILURE
    }
  }
}

#[php_function]
fn pasir_finish_request() -> bool {
  Context::from_server_context(SapiGlobals::get().server_context).finish_request()
}

#[cfg(test)]
pub(crate) mod tests {
  use std::collections::HashMap;
  use std::net::Ipv4Addr;
  use std::path::PathBuf;

  use hyper::Request;
  use rstest::rstest;

  use super::*;
  use crate::sapi::context::ContextBuilder;
  use crate::sapi::context::ContextSender;

  pub(crate) struct TestSapi(*mut SapiModule);

  impl TestSapi {
    pub(crate) fn new() -> Self {
      let sapi = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
        .read_cookies_function(read_cookies_test)
        .build()
        .unwrap()
        .into_raw();
      unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() };
      unsafe { pasir_sys::sapi_startup(sapi) };
      Self(sapi)
    }
  }

  impl Drop for TestSapi {
    fn drop(&mut self) {
      unsafe { pasir_sys::php_module_shutdown() };
      unsafe { pasir_sys::sapi_shutdown() };
      unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() };
    }
  }

  extern "C" fn read_cookies_test() -> *mut c_char {
    std::ptr::null_mut()
  }

  /// Macro to assert server variable values
  /// Usage: assert_var!(server_variables, REQUEST_URI, "/foo/bar")
  macro_rules! assert_var {
    ($server_variables:expr, $var:ident, $expected:expr) => {
      assert_eq!(
        $server_variables.get($var.to_str().unwrap()).unwrap().string().unwrap(),
        $expected
      );
    };
  }

  /// Test SAPI module creation
  /// This tests the core SAPI functionality of creating a new SAPI module instance
  #[test]
  fn test_sapi_new() {
    let sapi = Sapi::new(false, None);

    // Verify that the SAPI module pointer is not null
    assert!(!sapi.0.is_null(), "SAPI module should not be null after creation");

    // Verify that the SAPI module has the correct name and description
    unsafe {
      let sapi_module = *sapi.0;
      assert!(!sapi_module.name.is_null(), "SAPI module name should not be null");

      let name = CStr::from_ptr(sapi_module.name).to_string_lossy();
      assert_eq!(name, env!("CARGO_PKG_NAME"), "SAPI module name should match package name");

      // Verify callback functions are properly set
      assert!(sapi_module.startup.is_some(), "Startup function should be set");
      assert!(sapi_module.shutdown.is_some(), "Shutdown function should be set");
      assert!(sapi_module.ub_write.is_some(), "UB write function should be set");
      assert!(sapi_module.flush.is_some(), "Flush function should be set");
      assert!(sapi_module.send_header.is_some(), "Send header function should be set");
      assert!(sapi_module.read_post.is_some(), "Read post function should be set");
      assert!(sapi_module.read_cookies.is_some(), "Read cookies function should be set");
      assert!(
        sapi_module.register_server_variables.is_some(),
        "Register server variables function should be set"
      );
      assert!(sapi_module.log_message.is_some(), "Log message function should be set");
      assert!(sapi_module.get_request_time.is_some(), "Get request time function should be set");
      assert!(sapi_module.sapi_error.is_some(), "SAPI error function should be set");
    }
  }

  /// Test multiple SAPI instances can be created
  /// This ensures SAPI creation is properly isolated
  #[test]
  fn test_multiple_sapi_instances() {
    let sapi1 = Sapi::new(false, None);
    let sapi2 = Sapi::new(false, None);

    // Verify both instances have valid, different pointers
    assert!(!sapi1.0.is_null(), "First SAPI instance should be valid");
    assert!(!sapi2.0.is_null(), "Second SAPI instance should be valid");
    assert_ne!(sapi1.0, sapi2.0, "SAPI instances should have different pointers");
  }

  /// Test SAPI thread safety markers
  /// This verifies that Sapi implements Send and Sync correctly
  #[test]
  fn test_sapi_thread_safety() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    // These should compile without error due to our unsafe impl Send/Sync
    assert_send::<Sapi>();
    assert_sync::<Sapi>();
  }

  #[test]
  fn test_sapi_startup_shutdown() {
    let sapi = TestSapi::new();

    assert_eq!(ZEND_RESULT_CODE_SUCCESS, startup(sapi.0));
    assert_eq!(ZEND_RESULT_CODE_SUCCESS, shutdown(sapi.0))
  }

  #[rstest]
  #[case(false)]
  #[case::aborted(true)]
  fn test_deactivate(#[case] aborted: bool) {
    let _sapi = TestSapi::new();

    let (head_rx, _, context_sender) = ContextSender::receiver();
    let context = ContextBuilder::default().sender(context_sender).build();
    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.server_context = context.into_raw().cast();
    sapi_globals.sapi_started = true;
    drop(sapi_globals);

    unsafe { pasir_sys::php_output_startup() };
    if aborted {
      drop(head_rx);
    }
    assert_eq!(deactivate(), ZEND_RESULT_CODE_SUCCESS);
    assert!(SapiGlobals::get().server_context.is_null());
  }

  #[test]
  fn test_ub_write() {
    assert_eq!(ub_write(std::ptr::null_mut(), 0), 0);

    let _sapi = TestSapi::new();

    // assert `ub_write` without server context.
    assert_eq!(ub_write(c"Foo".as_ptr(), 3), 3);

    let (_head_rx, _body_rx, context_sender) = ContextSender::receiver();
    let context = ContextBuilder::default().sender(context_sender).build();

    SapiGlobals::get_mut().server_context = context.into_raw().cast();
    assert_eq!(ub_write(c"Foo".as_ptr(), 3), 3);

    unsafe { pasir_sys::php_output_startup() };
    let mut context = unsafe { Context::from_raw(SapiGlobals::get().server_context) };
    assert!(context.finish_request());
    assert_eq!(ub_write(c"Foo".as_ptr(), 3), 0);
  }

  #[tokio::test]
  async fn test_send_header() {
    send_header(std::ptr::null_mut(), std::ptr::null_mut());

    let _sapi = TestSapi::new();

    let (head_rx, _, context_sender) = ContextSender::receiver();
    let context = ContextBuilder::default().sender(context_sender).build();
    let context_raw = context.into_raw();
    let header = SapiHeader { header: c"Foo: Bar".as_ptr().cast_mut(), header_len: 8 };
    let header_raw = Box::into_raw(Box::new(header));
    send_header(header_raw, context_raw.cast());

    unsafe { pasir_sys::php_output_startup() };
    let mut context = unsafe { Context::from_raw(context_raw.cast()) };
    context.finish_request();

    let head = head_rx.await.unwrap();
    assert_eq!(head.headers.get("Foo"), Some(&HeaderValue::from_static("Bar")));
  }

  #[test]
  fn test_read_post() {
    let _sapi = TestSapi::new();

    let buffer = CString::default();
    let buffer_raw = buffer.into_raw();
    assert_eq!(read_post(buffer_raw, 0), 0);

    let request = Request::new(Bytes::from_static(b"Foo"));
    let context = ContextBuilder::default().request(request).build();
    SapiGlobals::get_mut().server_context = context.into_raw().cast();
    SapiGlobals::get_mut().request_info.content_length = 3;

    assert_eq!(read_post(buffer_raw, 1), 1);
    SapiGlobals::get_mut().read_post_bytes = 1;
    let buffer = unsafe { CString::from_raw(buffer_raw) };
    assert_eq!(buffer.as_c_str(), c"F");

    let buffer = CString::default();
    let buffer_raw = buffer.into_raw();
    assert_eq!(read_post(buffer_raw, 3), 2);
    SapiGlobals::get_mut().read_post_bytes = 3;
    let buffer = unsafe { CString::from_raw(buffer_raw) };
    assert_eq!(buffer.as_c_str(), c"oo");

    let buffer = CString::default();
    let buffer_raw = buffer.into_raw();
    assert_eq!(read_post(buffer_raw, 3), 0);

    let buffer = unsafe { CString::from_raw(buffer_raw) };
    assert_eq!(buffer.as_c_str(), c"");

    let _context = unsafe { Context::from_raw(SapiGlobals::get().server_context) };
  }

  #[test]
  fn test_read_cookies() {
    let _sapi = TestSapi::new();

    let request = Request::builder().header("Cookie", "foo=bar").body(Bytes::default()).unwrap();
    let context = ContextBuilder::default().request(request).build();
    SapiGlobals::get_mut().server_context = context.into_raw().cast();
    assert_eq!(unsafe { CString::from_raw(read_cookies()) }, CString::new("foo=bar").unwrap());

    let _context = unsafe { Context::from_raw(SapiGlobals::get().server_context) };
  }

  #[test]
  fn test_register_server_variables() -> anyhow::Result<()> {
    let _sapi = TestSapi::new();
    assert_eq!(
      unsafe { pasir_sys::php_module_startup(_sapi.0, std::ptr::null_mut()) },
      ZEND_RESULT_CODE_SUCCESS
    );
    assert_eq!(unsafe { pasir_sys::php_request_startup() }, ZEND_RESULT_CODE_SUCCESS);

    let localhost = Ipv4Addr::LOCALHOST;
    let root = PathBuf::from("/foo");
    let request = Request::builder()
      .header("Cookie", "foo=bar")
      .header("Host", localhost.to_string())
      .body(Bytes::default())?;
    let context = ContextBuilder::default()
      .root(root)
      .script_name("/index.php")
      .path_info("/foo/bar")
      .request(request)
      .build();

    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.request_info.request_uri = c"/foo/bar".as_ptr().cast_mut();
    sapi_globals.request_info.request_method = c"GET".as_ptr().cast_mut();
    sapi_globals.request_info.query_string = c"foo=bar".as_ptr().cast_mut();
    sapi_globals.server_context = context.into_raw().cast();
    drop(sapi_globals);

    let mut vars = Zval::new();
    let _ = vars.set_array(HashMap::<String, String>::new());
    assert!(vars.is_array());
    let vars_raw = Box::into_raw(Box::new(vars));
    register_server_variables(vars_raw);

    let zval = unsafe { Box::from_raw(vars_raw) };
    let vars = zval.array().unwrap();
    assert!(vars.get(SERVER_SOFTWARE.to_str()?).is_some());
    assert_var!(vars, REQUEST_URI, "/foo/bar");
    assert_var!(vars, REQUEST_METHOD, "GET");
    assert_var!(vars, QUERY_STRING, "foo=bar");
    assert_var!(vars, PHP_SELF, "/index.php/foo/bar");
    assert_var!(vars, SERVER_PROTOCOL, "HTTP/1.1");
    assert_var!(vars, DOCUMENT_ROOT, "/foo");
    assert_var!(vars, REMOTE_ADDR, localhost.to_string());
    assert_var!(vars, REMOTE_PORT, "0");
    assert_var!(vars, SCRIPT_FILENAME, "/foo/index.php");
    assert_var!(vars, SERVER_ADDR, localhost.to_string());
    assert_var!(vars, SERVER_PORT, "0");
    assert_var!(vars, SCRIPT_NAME, "/index.php");
    assert_var!(vars, PATH_INFO, "/foo/bar");
    assert_var!(vars, SERVER_NAME, localhost.to_string());
    assert_eq!(vars.get("HTTP_COOKIE").unwrap().string().unwrap(), "foo=bar");
    assert_eq!(vars.get("HTTP_HOST").unwrap().string().unwrap(), localhost.to_string());

    let _context = unsafe { Context::from_raw(SapiGlobals::get().server_context) };
    Ok(())
  }

  /// Test get_request_time callback
  /// This tests the request time functionality which is safe to call
  #[test]
  fn test_get_request_time() {
    let mut time: f64 = 0.0;
    let timestamp = SystemTime::UNIX_EPOCH.elapsed().unwrap().as_secs();
    let result = get_request_time(&mut time);

    // Should return success code
    assert_eq!(result, ZEND_RESULT_CODE_SUCCESS, "get_request_time should return success");
    unsafe { assert_eq!(time.to_int_unchecked::<u64>(), timestamp) }
  }
}
