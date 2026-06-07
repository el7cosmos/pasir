#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use ext_php_rs::embed::ServerContext as _;
use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::SapiModule;
use libc::LOG_DEBUG;
use pasir_sys::ZEND_RESULT_CODE;
use pasir_sys::ZEND_RESULT_CODE_SUCCESS;

use crate::context::ServerContext;
use crate::ext::SapiRequestInfoExt;

pub mod context;
pub mod error;
pub mod ext;
pub mod util;
pub mod variables;

pub trait Sapi: ext_php_rs::embed::Sapi {
  #[doc(hidden)]
  unsafe extern "C" fn startup(sapi: *mut SapiModule) -> ZEND_RESULT_CODE {
    unsafe { pasir_sys::php_module_startup(sapi, std::ptr::null_mut()) }
  }

  extern "C" fn shutdown(_sapi: *mut SapiModule) -> ZEND_RESULT_CODE {
    unsafe { pasir_sys::php_module_shutdown() };
    ZEND_RESULT_CODE_SUCCESS
  }

  extern "C" fn deactivate() -> ZEND_RESULT_CODE
  where
    Self::Context: ServerContext,
  {
    let sapi_globals = SapiGlobals::get();
    if !sapi_globals.sapi_started {
      return ZEND_RESULT_CODE_SUCCESS;
    }

    if sapi_globals.server_context.is_null() {
      return ZEND_RESULT_CODE_SUCCESS;
    }

    sapi_globals.request_info.free();

    let mut context = unsafe { Self::Context::from_raw(sapi_globals.server_context) };
    drop(sapi_globals);
    if !context.is_request_finished() && !context.finish_request() {
      Self::log_message("finish request failed", LOG_DEBUG);
      util::handle_abort_connection();
    }
    SapiGlobals::get_mut().server_context = std::ptr::null_mut();

    ZEND_RESULT_CODE_SUCCESS
  }

  fn php_info_as_text() -> bool {
    false
  }

  fn build_module() -> ext_php_rs::error::Result<ext_php_rs::embed::SapiModule>
  where
    Self: Sized,
    Self::Context: ServerContext,
  {
    let mut sapi_module = <Self as ext_php_rs::embed::Sapi>::build_module()?;

    sapi_module.startup = Some(Self::startup);
    sapi_module.shutdown = Some(Self::shutdown);
    sapi_module.deactivate = Some(Self::deactivate);
    sapi_module.sapi_error = Some(pasir_sys::zend_error);
    sapi_module.phpinfo_as_text = Self::php_info_as_text().into();

    Ok(sapi_module)
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
  use ext_php_rs::embed::RequestInfo;
  use ext_php_rs::embed::ServerVarRegistrar;
  use ext_php_rs::zend::SapiGlobals;
  use pasir_sys::ZEND_RESULT_CODE_SUCCESS;
  use rstest::rstest;

  use crate::Sapi;
  use crate::context::ServerContext;
  use crate::sapi_test_shutdown;
  use crate::sapi_test_startup;

  #[derive(Default)]
  struct TestServerContext {
    finish_request: bool,
  }

  impl ext_php_rs::embed::ServerContext for TestServerContext {
    fn init_request_info(&self, _info: &mut RequestInfo) {}

    fn read_post(&mut self, buf: &mut [u8]) -> usize {
      buf.len()
    }

    fn read_cookies(&self) -> Option<&str> {
      None
    }

    fn finish_request(&mut self) -> bool {
      self.finish_request
    }

    fn is_request_finished(&self) -> bool {
      false
    }
  }

  impl ServerContext for TestServerContext {
    fn register_server_variables(&self, _registrar: &mut ServerVarRegistrar) {}
  }

  struct TestSapi;

  impl TestSapi {}

  impl ext_php_rs::embed::Sapi for TestSapi {
    type Context = TestServerContext;

    fn name() -> &'static str {
      env!("CARGO_PKG_NAME")
    }

    fn pretty_name() -> &'static str {
      env!("CARGO_PKG_DESCRIPTION")
    }

    fn ub_write(_ctx: &mut Self::Context, buf: &[u8]) -> usize {
      buf.len()
    }

    fn log_message(_message: &str, _syslog_type: i32) {}
  }

  impl Sapi for TestSapi {}

  #[test]
  fn test_sapi_startup_shutdown() {
    let sapi = TestSapi::build_module().unwrap().into_raw();

    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() };
    unsafe { pasir_sys::sapi_startup(sapi) };

    assert_eq!(unsafe { TestSapi::startup(sapi) }, ZEND_RESULT_CODE_SUCCESS);
    assert_eq!(TestSapi::shutdown(sapi), ZEND_RESULT_CODE_SUCCESS);

    unsafe { pasir_sys::sapi_shutdown() };
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() };
  }

  #[rstest]
  #[case(false)]
  #[case::aborted(true)]
  fn test_deactivate(#[case] aborted: bool) {
    let sapi = TestSapi::build_module().unwrap().into_raw();
    unsafe { sapi_test_startup(sapi) }
    let context = TestServerContext { finish_request: aborted };

    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.server_context = context.into_raw().cast();
    sapi_globals.sapi_started = true;
    drop(sapi_globals);

    unsafe { pasir_sys::php_output_startup() };
    assert_eq!(TestSapi::deactivate(), ZEND_RESULT_CODE_SUCCESS);
    assert!(SapiGlobals::get().server_context.is_null());
    unsafe { sapi_test_shutdown() }
  }
}
