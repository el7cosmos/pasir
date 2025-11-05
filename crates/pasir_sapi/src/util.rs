use std::ffi::CStr;
use std::ffi::CString;

use ext_php_rs::types::Zval;
use ext_php_rs::zend::ExecutorGlobals;

pub fn handle_abort_connection() {
  if !ExecutorGlobals::get().bailout.is_null() {
    unsafe { pasir_sys::php_handle_aborted_connection() }
  }
}

/// Registers a PHP variable within the context of a given `Zval`.
///
/// # Safety
/// This function is marked as `unsafe` because it interacts with raw pointers and expects
/// the provided `vars` pointer to be valid and properly aligned. Calling this function with
/// invalid pointers or improperly managed memory may lead to undefined behavior.
///
/// # Type Parameters
/// - `Value`: A type that implements `Into<Vec<u8>>`. This allows conversion of the
///   provided value into a byte vector suitable for C-style strings.
///
/// # Parameters
/// - `name`: A reference to a `CStr` that represents the name of the PHP variable to be registered.
///   It must be a valid, null-terminated C string.
/// - `value`: The value to associate with the PHP variable. The value will be converted
///   into a `CString`. This ensures it is memory-safe and null-terminated before it is
///   passed to the underlying C function.
/// - `vars`: A raw mutable pointer to a `Zval`, which represents the PHP environment or context
///   within which the variable is being registered. This pointer must be valid for the lifetime
///   of the function and must point to a writable location.
///
/// # Panics
/// - The function will panic if the provided `value` contains an interior null byte, as it
///   would not be properly convertible into a `CString` in such a case.
///
/// # Example
/// ```rust
/// use std::ffi::{CStr, CString};
///
/// let name = CStr::from_bytes_with_nul(b"my_var\0").unwrap();
/// let value = "example_value";
/// let vars: *mut Zval = /* Assume this is a valid pointer to a Zval */;
///
/// unsafe {
///     register_variable(name, value, vars);
/// }
/// ```
///
/// # Notes
/// - The `pasir_sys::php_register_variable` function is assumed to be a binding to an
///   external C function. Its behavior, requirements, and constraints must align
///   with its documentation and guarantees.
///
/// # See Also
/// - `pasir_sys::php_register_variable` for more details on the underlying C function.
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
