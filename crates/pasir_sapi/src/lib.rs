use std::ffi::c_char;
use std::ffi::c_int;
use std::ops::Sub;
use std::time::SystemTime;

use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::SapiModule;
use libc::LOG_DEBUG;
use pasir_sys::ZEND_RESULT_CODE;
use pasir_sys::ZEND_RESULT_CODE_FAILURE;
use pasir_sys::ZEND_RESULT_CODE_SUCCESS;

use crate::context::ServerContext;

pub mod context;
pub mod ext;
pub mod util;
pub mod variables;

pub trait Sapi {
  type ServerContext<'a>: ServerContext;

  #[doc(hidden)]
  unsafe extern "C" fn startup(sapi: *mut SapiModule) -> ZEND_RESULT_CODE {
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

    let mut context = unsafe { Self::ServerContext::from_raw(sapi_globals.server_context) };
    drop(sapi_globals);
    if !context.is_request_finished() && !context.finish_request() {
      Self::log_message(c"finish request failed".as_ptr(), LOG_DEBUG);
      util::handle_abort_connection();
    }
    SapiGlobals::get_mut().server_context = std::ptr::null_mut();

    ZEND_RESULT_CODE_SUCCESS
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

    Self::ServerContext::from_server_context(sapi_globals.server_context).read_post(buffer, to_read)
  }

  extern "C" fn log_message(message: *const c_char, syslog_type_int: c_int);

  #[doc(hidden)]
  unsafe extern "C" fn get_request_time(time: *mut f64) -> ZEND_RESULT_CODE {
    let timestamp = SystemTime::UNIX_EPOCH.elapsed().expect("system time is before Unix epoch");
    unsafe { time.write(timestamp.as_secs_f64()) };
    ZEND_RESULT_CODE_SUCCESS
  }

  fn sapi_startup(&self) -> ZEND_RESULT_CODE
  where
    for<'a> &'a Self: Into<*mut SapiModule>,
  {
    let sapi_module = self.into();
    unsafe {
      let ini_entries = (*sapi_module).ini_entries;
      pasir_sys::sapi_startup(sapi_module);
      (*sapi_module).ini_entries = ini_entries;

      if let Some(startup) = (*sapi_module).startup {
        return startup(sapi_module);
      }
    }

    ZEND_RESULT_CODE_FAILURE
  }

  fn sapi_shutdown(&self)
  where
    for<'a> &'a Self: Into<*mut SapiModule>,
  {
    let sapi_module = self.into();
    unsafe {
      if let Some(shutdown) = (*sapi_module).shutdown {
        shutdown(sapi_module);
      }
      pasir_sys::sapi_shutdown();
    }
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use std::ffi::CString;
  use std::ffi::c_char;
  use std::ffi::c_int;
  use std::time::SystemTime;
  use std::time::SystemTimeError;

  use ext_php_rs::builders::SapiBuilder;
  use ext_php_rs::zend::SapiGlobals;
  use ext_php_rs::zend::SapiModule;
  use pasir_sys::ZEND_RESULT_CODE_FAILURE;
  use pasir_sys::ZEND_RESULT_CODE_SUCCESS;
  use rstest::rstest;

  use crate::Sapi;
  use crate::context::ServerContext;
  use crate::context::tests::TestServerContext;

  pub(crate) struct TestSapi(*mut SapiModule);

  impl TestSapi {
    pub(crate) fn new() -> Self {
      let sapi = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
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

  impl Sapi for TestSapi {
    type ServerContext<'a> = TestServerContext;

    extern "C" fn log_message(_message: *const c_char, _syslog_type_int: c_int) {}
  }

  impl From<&TestSapi> for *mut SapiModule {
    fn from(value: &TestSapi) -> Self {
      value.0
    }
  }

  #[test]
  fn test_sapi_startup_shutdown() {
    let sapi = TestSapi::new();

    assert_eq!(unsafe { TestSapi::startup(sapi.0) }, ZEND_RESULT_CODE_SUCCESS);
    assert_eq!(TestSapi::shutdown(sapi.0), ZEND_RESULT_CODE_SUCCESS);

    unsafe { (*sapi.0).startup = None };
    assert_eq!(sapi.sapi_startup(), ZEND_RESULT_CODE_FAILURE);
  }

  #[rstest]
  #[case(false)]
  #[case::aborted(true)]
  fn test_deactivate(#[case] aborted: bool) {
    let _sapi = TestSapi::new();
    let context = TestServerContext { finish_request: aborted };

    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.server_context = context.into_raw().cast();
    sapi_globals.sapi_started = true;
    drop(sapi_globals);

    unsafe { pasir_sys::php_output_startup() };
    assert_eq!(TestSapi::deactivate(), ZEND_RESULT_CODE_SUCCESS);
    assert!(SapiGlobals::get().server_context.is_null());
  }

  #[test]
  fn test_read_post() {
    let _sapi = TestSapi::new();

    let buffer = CString::default();
    let buffer_raw = buffer.into_raw();
    assert_eq!(TestSapi::read_post(buffer_raw, 0), 0);

    let context = TestServerContext::default();
    SapiGlobals::get_mut().server_context = context.into_raw().cast();
    SapiGlobals::get_mut().request_info.content_length = 3;
    assert_eq!(TestSapi::read_post(buffer_raw, 1), 1);

    SapiGlobals::get_mut().read_post_bytes = 3;
    assert_eq!(TestSapi::read_post(buffer_raw, 3), 0);

    let _ = unsafe { TestServerContext::from_raw(SapiGlobals::get().server_context) };
  }

  /// Test get_request_time callback
  /// This tests the request time functionality which is safe to call
  #[test]
  fn test_get_request_time() -> Result<(), SystemTimeError> {
    let mut time: f64 = 0.0;
    let timestamp = SystemTime::UNIX_EPOCH.elapsed()?.as_secs();
    let result = unsafe { TestSapi::get_request_time(&mut time) };

    // Should return success code
    assert_eq!(result, ZEND_RESULT_CODE_SUCCESS, "get_request_time should return success");
    unsafe { assert_eq!(time.to_int_unchecked::<u64>(), timestamp) }
    Ok(())
  }
}
