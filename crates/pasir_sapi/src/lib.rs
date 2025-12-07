#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

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
pub mod error;
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

/// Initializes the PHP SAPI module for testing purposes in a controlled environment.
///
/// This function is marked as `unsafe` because it involves interactions with
/// raw pointers and external FFI (Foreign Function Interface) calls, requiring
/// careful handling to avoid undefined behavior. It should only be used in a test
/// configuration, as indicated by the `#[cfg(test)]` attribute.
///
/// # Parameters
/// - `sapi`: A mutable raw pointer to a `SapiModule` structure, representing the SAPI module to be initialized.
///
/// # Safety
/// This function executes unsafe external FFI functions and directly operates on
/// raw pointers, which can lead to undefined behavior, memory corruption, or application
/// crashes if misused. Make sure that the provided `sapi` pointer is valid, properly allocated,
/// and initialized before calling this function. The caller is responsible for ensuring safety.
///
/// # Functionality
/// 1. Calls the `ext_php_rs::embed::ext_php_rs_sapi_startup` function to initialize the ext-php-rs framework.
/// 2. Calls the `pasir_sys::sapi_startup` function to set up the given SAPI module (`sapi`).
/// 3. Calls the `pasir_sys::php_module_startup` function to complete the initialization of the PHP modules.
///
/// # Example
/// ```rust
/// #[cfg(test)]
/// unsafe {
///     let mut sapi: SapiModule = SapiModule::default(); // Assuming defaultable SapiModule for example.
///     sapi_test_startup(&mut sapi);
/// }
/// ```
///
/// # Caveats
/// - This function is only available in test builds as it is conditionally compiled
///   with the `#[cfg(test)]` directive.
/// - Do not call this function outside a controlled testing environment.
///
/// # See Also
/// - `ext_php_rs::embed::ext_php_rs_sapi_startup`
/// - `pasir_sys::sapi_startup`
/// - `pasir_sys::php_module_startup`
#[cfg(test)]
pub unsafe fn sapi_test_startup(sapi: *mut SapiModule) {
  unsafe {
    ext_php_rs::embed::ext_php_rs_sapi_startup();
    pasir_sys::sapi_startup(sapi);
    pasir_sys::php_module_startup(sapi, std::ptr::null_mut());
  }
}

/// Shuts down the embedded SAPI and cleans up resources used during testing.
///
/// # Safety
/// This function is `unsafe` because it directly interacts with and shuts down
/// the underlying PHP SAPI subsystem, which may leave the program in an
/// undefined state if not used correctly. It is intended to be used only in
/// test environments and assumes the SAPI has been properly initialized before
/// calling this function.
///
/// # Details
/// The function sequentially shuts down the PHP module and the SAPI layer
/// through calls to the underlying `pasir_sys` library. It also finalizes the
/// embedded Rust-PHP integration via calls to `ext_php_rs`. These steps
/// ensure a clean shutdown of the testing environment.
///
/// # Usage
/// This function should only be called within a test configuration to
/// clean up resources used during testing of PHP integration with Rust.
/// Ensure that no other parts of the program depend on the SAPI subsystem
/// after invoking this function.
///
/// # Example
/// ```
/// fn test_sapi_shutdown() {
///     unsafe {
///         // perform the necessary SAPI initialization here
///
///         sapi_test_shutdown();
///     }
/// }
/// ```
///
/// # Notes
/// - Misuse of this function can result in undefined behavior.
/// - Ensure that all cleanup steps are correctly performed, and no later
///   calls to PHP APIs are made after shutdown.
#[cfg(test)]
pub unsafe fn sapi_test_shutdown() {
  unsafe {
    pasir_sys::php_module_shutdown();
    pasir_sys::sapi_shutdown();
    ext_php_rs::embed::ext_php_rs_sapi_shutdown();
  }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
  use std::ffi::CString;
  use std::ffi::NulError;
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
  use crate::sapi_test_shutdown;

  #[derive(Default)]
  struct TestServerContext {
    finish_request: bool,
  }

  impl ServerContext for TestServerContext {
    fn init_sapi_globals(&mut self) -> Result<(), NulError> {
      Ok(())
    }

    fn read_post(&mut self, _buffer: *mut c_char, to_read: usize) -> usize {
      to_read
    }

    fn is_request_finished(&self) -> bool {
      false
    }

    fn finish_request(&mut self) -> bool {
      self.finish_request
    }
  }

  struct TestSapi(*mut SapiModule);

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
      unsafe { sapi_test_shutdown() };
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
  }

  #[test]
  fn test_sapi_startup_failure() {
    let sapi = TestSapi::new();
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
