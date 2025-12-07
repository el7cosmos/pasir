use std::ffi::NulError;
use std::ffi::c_char;
use std::ffi::c_void;
use std::fmt::Debug;
use std::panic::RefUnwindSafe;
use std::path::Path;

use ext_php_rs::embed::Embed;
use ext_php_rs::embed::EmbedError;
use ext_php_rs::zend::SapiGlobals;
use pasir_sys::ZEND_RESULT_CODE_FAILURE;

use crate::error::ExecutePhpError;
use crate::free_raw_cstring_mut;

pub trait ServerContext: Sized {
  #[must_use = "losing the pointer will leak memory"]
  fn into_raw(self) -> *mut Self {
    Box::into_raw(Box::new(self))
  }

  /// Constructs a `Box<Self>` from a raw pointer.
  ///
  /// # Safety
  ///
  /// This function is unsafe because it transfers the ownership of the raw pointer to the resulting
  /// `Box<Self>`. The caller must ensure that the pointer was allocated by the same allocator and
  /// corresponds to a `Self` type. Additionally, the pointer must not be null, and it must not have
  /// been previously freed.
  ///
  /// # Important
  /// The returned `Box<Self>` assumes ownership of the raw pointer. To properly release the resources
  /// associated with the raw pointer when the `Context` is no longer needed, you must call
  /// `drop(Context::from_raw(ptr))`, if you intend to drop the `Context`.
  ///
  /// # Parameters
  /// - `ptr`: A raw pointer to a `Self` type. This pointer must be valid, properly aligned,
  ///   and allocated by the same allocator being used here.
  ///
  /// # Returns
  /// A `Box<Self>` that takes ownership of the memory referenced by `ptr`.
  ///
  /// # Example
  /// ```
  /// use std::ffi::c_void;
  ///
  /// // Example usage, though unsafe to use without guarantees
  /// unsafe {
  ///     let context_ptr: *mut c_void = some_context_as_ptr();
  ///     let context_box = Context::from_raw(context_ptr);
  /// }
  /// ```
  ///
  /// # Panics
  /// This function does not explicitly panic, but improper usage of this function (e.g., passing an
  /// invalid, null, or double-freed pointer) will cause **undefined behavior**, which may manifest
  /// as crashes or data corruption.
  ///
  #[must_use = "call `drop(Context::from_raw(ptr))` if you intend to drop the `Context`"]
  unsafe fn from_raw(ptr: *mut c_void) -> Box<Self> {
    unsafe { Box::from_raw(ptr.cast()) }
  }

  fn from_server_context<'a>(server_context: *mut c_void) -> &'a mut Self {
    let context = server_context.cast();
    unsafe { &mut *context }
  }

  fn init_sapi_globals(&mut self) -> Result<(), NulError>;

  #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, embed_error_handler), err))]
  fn execute_php<P, F>(mut self, script: P, embed_error_handler: F) -> Result<(), ExecutePhpError>
  where
    P: AsRef<Path> + Debug + RefUnwindSafe,
    F: Fn(EmbedError) + RefUnwindSafe,
  {
    if let Err(err) = self.init_sapi_globals() {
      return Err(ExecutePhpError::from(err));
    }
    SapiGlobals::get_mut().server_context = self.into_raw().cast();

    if unsafe { pasir_sys::php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
      return Err(ExecutePhpError::RequestStartupFailed);
    }

    let catch = ext_php_rs::zend::try_catch_first(|| {
      if let Err(e) = Embed::run_script(&script)
        && e.is_bailout()
      {
        embed_error_handler(e);
      }
    });

    #[cfg(php84)]
    unsafe {
      pasir_sys::zend_shutdown_strtod()
    };
    unsafe { pasir_sys::php_request_shutdown(std::ptr::null_mut()) };
    let mut request_info = SapiGlobals::get().request_info;
    free_raw_cstring_mut!(request_info, path_translated);

    if catch.is_err() {
      return Err(ExecutePhpError::Bailout);
    }

    Ok(())
  }

  fn read_post(&mut self, buffer: *mut c_char, to_read: usize) -> usize;

  fn is_request_finished(&self) -> bool;

  fn finish_request(&mut self) -> bool;
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
  use std::ffi::CString;
  use std::ffi::NulError;
  use std::ffi::c_char;
  use std::ffi::c_int;
  use std::path::PathBuf;

  use ext_php_rs::builders::SapiBuilder;
  use ext_php_rs::zend::SapiModule;
  use rstest::rstest;

  use crate::Sapi;
  use crate::context::ServerContext;
  use crate::error::ExecutePhpError;

  #[rstest]
  #[case::init_sapi_globals_ok(true)]
  #[case::init_sapi_globals_err(false)]
  fn test_execute_php(#[case] init_sapi_globals_ok: bool) {
    struct TestContext {
      init_sapi_globals_result: Result<(), NulError>,
    }

    impl ServerContext for TestContext {
      fn init_sapi_globals(&mut self) -> Result<(), NulError> {
        self.init_sapi_globals_result.clone()
      }

      fn read_post(&mut self, _buffer: *mut c_char, _to_read: usize) -> usize {
        todo!()
      }

      fn is_request_finished(&self) -> bool {
        todo!()
      }

      fn finish_request(&mut self) -> bool {
        todo!()
      }
    }

    struct TestSapi(*mut SapiModule);

    impl TestSapi {
      fn new() -> Self {
        extern "C" fn read_cookies() -> *mut c_char {
          std::ptr::null_mut()
        }

        extern "C" fn ub_write(_str: *const c_char, str_length: usize) -> usize {
          str_length
        }

        let sapi = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
          .ub_write_function(ub_write)
          .read_cookies_function(read_cookies)
          .build()
          .unwrap()
          .into_raw();
        unsafe { crate::sapi_test_startup(sapi) };
        Self(sapi)
      }
    }

    impl Drop for TestSapi {
      fn drop(&mut self) {
        unsafe { crate::sapi_test_shutdown() };
      }
    }

    impl Sapi for TestSapi {
      type ServerContext<'a> = TestContext;

      extern "C" fn log_message(_message: *const c_char, _syslog_type_int: c_int) {}
    }

    impl From<&TestSapi> for *mut SapiModule {
      fn from(value: &TestSapi) -> Self {
        value.0
      }
    }

    let byte: &[u8] = match init_sapi_globals_ok {
      true => b"foo",
      false => b"f\0oo",
    };
    let init_sapi_globals_result = CString::new(byte).map(|_| ());

    let _sapi = TestSapi::new();
    let context = TestContext {
      init_sapi_globals_result: init_sapi_globals_result.clone(),
    };
    let result = context.execute_php(PathBuf::new(), |_| {});
    assert_eq!(result, init_sapi_globals_result.map_err(ExecutePhpError::from));
  }
}
