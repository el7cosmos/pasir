use std::ffi::c_void;

pub trait ServerContext: Sized {
  #[must_use = "losing the pointer will leak memory"]
  fn into_raw(self) -> *mut Self {
    Box::into_raw(Box::new(self))
  }

  /// # Safety
  ///
  /// This function should only be called by the end of the request before finishing the request.
  #[must_use = "call `drop(Context::from_raw(ptr))` if you intend to drop the `Context`"]
  unsafe fn from_raw(ptr: *mut c_void) -> Box<Self> {
    unsafe { Box::from_raw(ptr.cast()) }
  }

  fn from_server_context<'a>(server_context: *mut c_void) -> &'a mut Self {
    let context = server_context.cast();
    unsafe { &mut *context }
  }

  fn is_request_finished(&self) -> bool;

  fn finish_request(&mut self) -> bool;
}

#[cfg(test)]
pub(crate) mod tests {
  use crate::context::ServerContext;

  #[derive(Default)]
  pub(crate) struct TestServerContext {
    pub(crate) finish_request: bool,
  }

  impl ServerContext for TestServerContext {
    fn is_request_finished(&self) -> bool {
      false
    }

    fn finish_request(&mut self) -> bool {
      self.finish_request
    }
  }
}
