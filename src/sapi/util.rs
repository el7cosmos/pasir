use ext_php_rs::ffi::php_register_variable;
use ext_php_rs::types::Zval;
use std::ffi::CString;

pub(crate) fn parse_header(header: &str) -> Option<(String, String)> {
  if let Some(idx) = header.find(':') {
    let (name, value) = header.split_at(idx);
    let value = value.trim_start_matches(':').trim();
    Some((name.trim().to_string(), value.to_string()))
  } else {
    None
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
