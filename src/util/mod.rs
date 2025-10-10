pub(crate) mod response_ext;

#[macro_export]
macro_rules! free_raw_cstring {
  ($struct:expr, $field:ident) => {
    if !$struct.$field.is_null() {
      unsafe { drop(CString::from_raw($struct.$field.cast_mut())) };
      $struct.$field = std::ptr::null();
    }
  };
}

#[macro_export]
macro_rules! free_raw_cstring_mut {
  ($struct:expr, $field:ident) => {
    if !$struct.$field.is_null() {
      unsafe { drop(CString::from_raw($struct.$field)) };
      $struct.$field = std::ptr::null_mut();
    }
  };
}
