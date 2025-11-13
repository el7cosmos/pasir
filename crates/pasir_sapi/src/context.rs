use std::ffi::c_char;
use std::ffi::c_void;

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

  fn read_post(&mut self, buffer: *mut c_char, to_read: usize) -> usize;

  fn is_request_finished(&self) -> bool;

  fn finish_request(&mut self) -> bool;
}

#[cfg(test)]
pub(crate) mod tests {
  use std::ffi::c_char;

  use crate::context::ServerContext;

  #[derive(Default)]
  pub(crate) struct TestServerContext {
    pub(crate) finish_request: bool,
  }

  impl ServerContext for TestServerContext {
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
}
