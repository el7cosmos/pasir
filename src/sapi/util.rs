use ext_php_rs::ffi::{php_handle_aborted_connection, php_register_variable};
use ext_php_rs::types::Zval;
use ext_php_rs::zend::ExecutorGlobals;
use std::ffi::CString;

pub(crate) fn handle_abort_connection() {
  if !ExecutorGlobals::get().bailout.is_null() {
    unsafe { php_handle_aborted_connection() }
  }
}

pub(crate) fn register_variable<Name: Into<Vec<u8>>, Value: Into<Vec<u8>>>(
  name: Name,
  value: Value,
  vars: *mut Zval,
) {
  unsafe {
    php_register_variable(
      CString::new(name).unwrap().into_raw(),
      CString::new(value).unwrap().into_raw(),
      vars,
    );
  }
}
