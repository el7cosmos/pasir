use ext_php_rs::ffi::{php_handle_aborted_connection, php_register_variable};
use ext_php_rs::types::Zval;
use ext_php_rs::zend::ExecutorGlobals;
use std::ffi::{CStr, CString};

pub(crate) fn handle_abort_connection() {
  if !ExecutorGlobals::get().bailout.is_null() {
    unsafe { php_handle_aborted_connection() }
  }
}

pub(crate) fn register_variable<Value: Into<Vec<u8>>>(name: &CStr, value: Value, vars: *mut Zval) {
  unsafe {
    let c_value = CString::new(value).unwrap();
    php_register_variable(name.as_ptr(), c_value.as_ptr(), vars);
  }
}
