use std::ffi::CStr;
use std::ffi::CString;

use ext_php_rs::types::Zval;
use ext_php_rs::zend::ExecutorGlobals;

pub(crate) fn handle_abort_connection() {
  if !ExecutorGlobals::get().bailout.is_null() {
    unsafe { pasir::ffi::php_handle_aborted_connection() }
  }
}

pub(crate) fn register_variable<Value: Into<Vec<u8>>>(name: &CStr, value: Value, vars: *mut Zval) {
  unsafe {
    let c_value = CString::new(value).unwrap();
    pasir::ffi::php_register_variable(name.as_ptr(), c_value.as_ptr(), vars);
  }
}
