use std::ffi::CStr;
use std::ffi::CString;

use ext_php_rs::types::Zval;
use ext_php_rs::zend::ExecutorGlobals;

pub fn handle_abort_connection() {
  if !ExecutorGlobals::get().bailout.is_null() {
    unsafe { pasir_sys::php_handle_aborted_connection() }
  }
}

/// # Safety
///
/// This function should only be called inside sapi's `register_server_variables` function.
pub unsafe fn register_variable<Value: Into<Vec<u8>>>(name: &CStr, value: Value, vars: *mut Zval) {
  let c_value = CString::new(value).unwrap();
  unsafe { pasir_sys::php_register_variable(name.as_ptr(), c_value.as_ptr(), vars) };
}

#[macro_export]
macro_rules! free_raw_cstring {
  ($struct:expr, $field:ident) => {
    if !$struct.$field.is_null() {
      unsafe { drop(std::ffi::CString::from_raw($struct.$field.cast_mut())) };
      $struct.$field = std::ptr::null();
    }
  };
}

#[macro_export]
macro_rules! free_raw_cstring_mut {
  ($struct:expr, $field:ident) => {
    if !$struct.$field.is_null() {
      unsafe { drop(std::ffi::CString::from_raw($struct.$field)) };
      $struct.$field = std::ptr::null_mut();
    }
  };
}
