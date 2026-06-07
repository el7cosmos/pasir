use ext_php_rs::zend::ExecutorGlobals;

pub fn handle_abort_connection() {
  if !ExecutorGlobals::get().bailout.is_null() {
    unsafe { pasir_sys::php_handle_aborted_connection() }
  }
}

#[macro_export]
macro_rules! free_raw_cstring {
  ($struct:expr, $field:ident) => {
    if !$struct.$field.is_null() {
      unsafe { drop(std::ffi::CString::from_raw($struct.$field.cast_mut())) };
    }
  };
}

#[macro_export]
macro_rules! free_raw_cstring_mut {
  ($struct:expr, $field:ident) => {
    if !$struct.$field.is_null() {
      unsafe { drop(std::ffi::CString::from_raw($struct.$field)) };
    }
  };
}
