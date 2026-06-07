use std::ffi::c_void;
use std::fmt::Debug;
use std::panic::RefUnwindSafe;
use std::path::Path;

use ext_php_rs::embed::Embed;
use ext_php_rs::embed::EmbedError;
use ext_php_rs::embed::RequestInfo;
use ext_php_rs::embed::ServerVarRegistrar;
use ext_php_rs::zend::SapiGlobals;
use pasir_sys::ZEND_RESULT_CODE_FAILURE;

use crate::error::ExecutePhpError;
use crate::ext::SapiRequestInfoExt;
use crate::free_raw_cstring_mut;

pub trait ServerContext: Sized + ext_php_rs::embed::ServerContext {
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

  fn register_server_variables(&self, registrar: &mut ServerVarRegistrar);

  #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, embed_error_handler), err))]
  fn execute_php<P, F>(self, script: P, embed_error_handler: F) -> Result<(), ExecutePhpError>
  where
    P: AsRef<Path> + Debug + RefUnwindSafe,
    F: Fn(EmbedError) + RefUnwindSafe,
  {
    let mut request_info = RequestInfo::default();
    self.init_request_info(&mut request_info);

    let mut sapi_globals = SapiGlobals::get_mut();
    sapi_globals.request_info.populate_from_request_info(request_info);
    sapi_globals.server_context = self.into_raw().cast();
    drop(sapi_globals);

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
    free_raw_cstring_mut!(SapiGlobals::get().request_info, path_translated);

    if catch.is_err() {
      return Err(ExecutePhpError::Bailout);
    }

    Ok(())
  }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
  use ext_php_rs::embed::RequestInfo;
  use ext_php_rs::embed::ServerVarRegistrar;
  use rstest::rstest;

  use crate::Sapi;
  use crate::context::ServerContext;

  #[rstest]
  fn test_execute_php() {
    struct TestContext {}

    impl ext_php_rs::embed::ServerContext for TestContext {
      fn init_request_info(&self, _info: &mut RequestInfo) {}

      fn read_post(&mut self, _buf: &mut [u8]) -> usize {
        0
      }

      fn read_cookies(&self) -> Option<&str> {
        None
      }

      fn finish_request(&mut self) -> bool {
        true
      }

      fn is_request_finished(&self) -> bool {
        false
      }
    }

    impl ServerContext for TestContext {
      fn register_server_variables(&self, _registrar: &mut ServerVarRegistrar) {}
    }

    struct TestSapi {}

    impl ext_php_rs::embed::Sapi for TestSapi {
      type Context = TestContext;

      fn name() -> &'static str {
        ""
      }

      fn pretty_name() -> &'static str {
        ""
      }

      fn ub_write(_ctx: &mut Self::Context, buf: &[u8]) -> usize {
        assert_eq!(buf, b"foo");
        buf.len()
      }

      fn log_message(_message: &str, _syslog_type: i32) {}
    }

    impl Sapi for TestSapi {}

    let sapi = TestSapi::build_module().unwrap().into_raw();

    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() };
    unsafe { pasir_sys::sapi_startup(sapi) };
    unsafe { pasir_sys::php_module_startup(sapi, std::ptr::null_mut()) };

    let context = TestContext {};
    let result = context.execute_php(std::env::current_dir().unwrap().join("tests/fixtures/script.php"), |e| {
      panic!("{:?}", e);
    });
    assert!(result.is_ok());

    unsafe { pasir_sys::php_module_shutdown() };
    unsafe { pasir_sys::sapi_shutdown() };
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() };
  }
}
