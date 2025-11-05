use std::ffi::c_char;
use std::ffi::c_int;
use std::time::SystemTime;

use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::SapiModule;
use libc::LOG_DEBUG;
use pasir_sys::ZEND_RESULT_CODE;
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

  extern "C" fn log_message(message: *const c_char, syslog_type_int: c_int);

  #[doc(hidden)]
  unsafe extern "C" fn get_request_time(time: *mut f64) -> ZEND_RESULT_CODE {
    let timestamp = SystemTime::UNIX_EPOCH.elapsed().expect("system time is before Unix epoch");
    unsafe { time.write(timestamp.as_secs_f64()) };
    ZEND_RESULT_CODE_SUCCESS
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use std::ffi::c_char;
  use std::ffi::c_int;
  use std::time::SystemTime;
  use std::time::SystemTimeError;

  use ext_php_rs::builders::SapiBuilder;
  use ext_php_rs::zend::SapiGlobals;
  use ext_php_rs::zend::SapiModule;
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

    extern "C" fn log_message(message: *const c_char, syslog_type_int: c_int) {}
  }

  #[test]
  fn test_sapi_startup_shutdown() {
    let sapi = TestSapi::new();

    assert_eq!(unsafe { TestSapi::startup(sapi.0) }, ZEND_RESULT_CODE_SUCCESS);
    assert_eq!(TestSapi::shutdown(sapi.0), ZEND_RESULT_CODE_SUCCESS)
  }

  #[rstest]
  #[case(false)]
  #[case::aborted(true)]
  fn test_deactivate(#[case] aborted: bool) {
    let _sapi = TestSapi::new();
    let mut context = TestServerContext::default();
    context.finish_request = aborted;

    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.server_context = context.into_raw().cast();
    sapi_globals.sapi_started = true;
    drop(sapi_globals);

    unsafe { pasir_sys::php_output_startup() };
    assert_eq!(TestSapi::deactivate(), ZEND_RESULT_CODE_SUCCESS);
    assert!(SapiGlobals::get().server_context.is_null());
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
