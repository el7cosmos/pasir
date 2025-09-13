pub(crate) mod context;
mod ext;
mod util;
mod variables;

use crate::sapi::context::Context;
use crate::sapi::util::handle_abort_connection;
use crate::sapi::util::register_variable;
use crate::sapi::variables::*;
use bytes::Bytes;
use bytes::BytesMut;
use ext_php_rs::builders::ModuleBuilder;
use ext_php_rs::builders::SapiBuilder;
use ext_php_rs::ffi::ZEND_RESULT_CODE_FAILURE;
use ext_php_rs::ffi::ZEND_RESULT_CODE_SUCCESS;
use ext_php_rs::ffi::php_module_shutdown;
use ext_php_rs::ffi::php_module_startup;
use ext_php_rs::ffi::php_register_variable;
use ext_php_rs::ffi::sapi_shutdown;
use ext_php_rs::ffi::sapi_startup;
use ext_php_rs::ffi::zend_error;
use ext_php_rs::php_function;
use ext_php_rs::php_module;
use ext_php_rs::types::Zval;
use ext_php_rs::wrap_function;
use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::SapiHeader;
use ext_php_rs::zend::SapiModule;
use headers::HeaderMapExt;
use headers::Host;
use hyper::Uri;
use hyper::header::{HeaderName, HeaderValue};
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::ffi::c_int;
use std::ffi::c_void;
use std::ops::Sub;
use std::str::FromStr;
use std::time::SystemTime;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::instrument;
use tracing::warn;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sapi(pub(crate) *mut SapiModule);

unsafe impl Send for Sapi {}
unsafe impl Sync for Sapi {}

impl Sapi {
  pub(crate) fn new(php_info_as_text: bool, ini_entries: Option<String>) -> Self {
    let mut builder = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
      .startup_function(startup)
      .shutdown_function(shutdown)
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

    let mut sapi_module = builder.build().unwrap();
    sapi_module.phpinfo_as_text = php_info_as_text as c_int;
    sapi_module.sapi_error = Some(zend_error);
    Self(sapi_module.into_raw())
  }

  pub(crate) fn startup(self) -> Result<(), ()> {
    unsafe {
      let ini_entries = match (*self.0).ini_entries.is_null() {
        true => None,
        false => Some((*self.0).ini_entries),
      };
      sapi_startup(self.0);
      if let Some(entries) = ini_entries {
        (*self.0).ini_entries = entries
      }

      let startup = (*self.0).startup.expect("startup function is null");
      match startup(self.0) {
        ZEND_RESULT_CODE_SUCCESS => Ok(()),
        ZEND_RESULT_CODE_FAILURE => Err(()),
        _ => Err(()),
      }
    }
  }

  pub(crate) fn shutdown(self) {
    unsafe {
      if let Some(shutdown) = (*self.0).shutdown {
        shutdown(self.0);
      }
      sapi_shutdown()
    }
  }
}

extern "C" fn startup(sapi: *mut SapiModule) -> c_int {
  unsafe { php_module_startup(sapi, get_module()) }
}

extern "C" fn shutdown(_sapi: *mut SapiModule) -> c_int {
  unsafe { php_module_shutdown() };
  ZEND_RESULT_CODE_SUCCESS as c_int
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
  if header.is_null() || server_context.is_null() {
    return;
  }

  let sapi_header = unsafe { *header };
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
  unsafe { buffer.copy_from(bytes.as_ptr().cast::<c_char>(), bytes.len()) }
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
      php_register_variable(REQUEST_URI.as_ptr(), request_info.request_uri.cast_const(), vars);
    }
  }
  if !request_info.request_method.is_null() {
    unsafe {
      php_register_variable(REQUEST_METHOD.as_ptr(), request_info.request_method, vars);
    }
  }
  if !request_info.query_string.is_null() {
    unsafe {
      php_register_variable(QUERY_STRING.as_ptr(), request_info.query_string.cast_const(), vars);
    }
  }

  if sapi_globals.server_context.is_null() {
    return;
  }

  let context = Context::from_server_context(sapi_globals.server_context);
  let root = context.root().to_str().unwrap_or_default();
  let script_name = context.route().script_name();
  let path_info = context.route().path_info();
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
fn fastcgi_finish_request() -> bool {
  Context::from_server_context(SapiGlobals::get().server_context).finish_request()
}

unsafe extern "C" fn request_shutdown(_type: i32, _module_number: i32) -> i32 {
  let mut request_info = SapiGlobals::get().request_info;

  free_ptr(&mut request_info.request_method.cast_mut());
  free_ptr(&mut request_info.query_string);
  free_ptr(&mut request_info.path_translated);
  free_ptr(&mut request_info.request_uri);
  free_ptr(&mut request_info.content_type.cast_mut());
  free_ptr(&mut request_info.cookie_data);

  ZEND_RESULT_CODE_SUCCESS
}

// Helper to free and null a pointer we allocated with into_raw()
fn free_ptr(ptr: &mut *mut std::os::raw::c_char) {
  if !ptr.is_null() {
    let _ = unsafe { CString::from_raw(*ptr) };
    *ptr = std::ptr::null_mut();
  }
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
  module
    .function(wrap_function!(fastcgi_finish_request))
    .request_shutdown_function(request_shutdown)
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use ext_php_rs::embed::ext_php_rs_sapi_shutdown;
  use ext_php_rs::embed::ext_php_rs_sapi_startup;

  pub(crate) struct TestSapi(*mut SapiModule);

  impl TestSapi {
    pub(crate) fn new() -> Self {
      let sapi = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
        .build()
        .unwrap()
        .into_raw();
      unsafe { ext_php_rs_sapi_startup() }
      unsafe { sapi_startup(sapi) }
      Self(sapi)
    }
  }

  impl Drop for TestSapi {
    fn drop(&mut self) {
      unsafe { sapi_shutdown() }
      unsafe { ext_php_rs_sapi_shutdown() }
    }
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

  #[test]
  fn test_sapi_startup_shutdown() {
    let sapi = TestSapi::new();

    assert_eq!(ZEND_RESULT_CODE_SUCCESS, startup(sapi.0));
    assert_eq!(ZEND_RESULT_CODE_SUCCESS, shutdown(sapi.0))
  }

  #[test]
  fn test_ub_write() {
    assert_eq!(ub_write(std::ptr::null_mut(), 0), 0);
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
