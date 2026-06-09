use std::ffi::CString;

use ext_php_rs::embed::RequestInfo;
use ext_php_rs::ffi::sapi_request_info;

use crate::free_raw_cstring;
use crate::free_raw_cstring_mut;

pub trait SapiRequestInfoExt {
  fn populate_from_request_info(&mut self, request_info: RequestInfo);

  fn free(self);
}

impl SapiRequestInfoExt for sapi_request_info {
  fn populate_from_request_info(&mut self, request_info: RequestInfo) {
    if let Some(request_method) = request_info.request_method.and_then(|s| CString::new(s).ok()) {
      self.request_method = request_method.into_raw().cast_const();
    }

    if let Some(query_string) = request_info.query_string.and_then(|s| CString::new(s).ok()) {
      self.query_string = query_string.into_raw();
    }

    if let Some(request_uri) = request_info.request_uri.and_then(|s| CString::new(s).ok()) {
      self.request_uri = request_uri.into_raw();
    }

    if let Some(path_translated) = request_info.path_translated.and_then(|s| CString::new(s).ok()) {
      self.path_translated = path_translated.into_raw();
    }

    if let Some(content_type) = request_info.content_type.and_then(|s| CString::new(s).ok()) {
      self.content_type = content_type.into_raw().cast_const();
    }

    self.content_length = request_info.content_length;
    self.proto_num = request_info.proto_num as std::os::raw::c_int;

    if let Some(auth_user) = request_info.auth_user.and_then(|s| CString::new(s).ok()) {
      self.auth_user = auth_user.into_raw();
    }

    if let Some(auth_password) = request_info.auth_password.and_then(|s| CString::new(s).ok()) {
      self.auth_password = auth_password.into_raw();
    }
  }

  fn free(self) {
    free_raw_cstring!(self, request_method);
    free_raw_cstring_mut!(self, query_string);
    free_raw_cstring_mut!(self, cookie_data);
    free_raw_cstring_mut!(self, request_uri);
    free_raw_cstring!(self, content_type);
    free_raw_cstring_mut!(self, auth_user);
    free_raw_cstring_mut!(self, auth_password);
  }
}

#[cfg(test)]
mod test {
  use ext_php_rs::embed::RequestInfo;
  use ext_php_rs::ffi::sapi_request_info;

  use crate::ext::SapiRequestInfoExt;
  use crate::free_raw_cstring_mut;

  #[test]
  fn test_sapi_request_info_populate_from_request_info() {
    let request_info = RequestInfo {
      request_method: Some("GET".to_string()),
      query_string: Some("foo=bar".to_string()),
      request_uri: Some("http://localhost".to_string()),
      path_translated: Some("/".to_string()),
      content_type: Some("text/html".to_string()),
      content_length: 1,
      proto_num: 1100,
      auth_user: Some("user".to_string()),
      auth_password: Some("password".to_string()),
    };

    let mut sapi_request_info = sapi_request_info {
      request_method: Default::default(),
      query_string: Default::default(),
      cookie_data: Default::default(),
      content_length: Default::default(),
      path_translated: Default::default(),
      request_uri: Default::default(),
      request_body: Default::default(),
      content_type: Default::default(),
      headers_only: Default::default(),
      no_headers: Default::default(),
      headers_read: Default::default(),
      post_entry: Default::default(),
      content_type_dup: Default::default(),
      auth_user: Default::default(),
      auth_password: Default::default(),
      auth_digest: Default::default(),
      argv0: Default::default(),
      current_user: Default::default(),
      current_user_length: Default::default(),
      argc: Default::default(),
      argv: Default::default(),
      proto_num: Default::default(),
    };
    sapi_request_info.populate_from_request_info(request_info);

    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.request_method) }, c"GET");
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.query_string) }, c"foo=bar");
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.request_uri) }, c"http://localhost");
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.path_translated) }, c"/");
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.content_type) }, c"text/html");
    assert_eq!(sapi_request_info.content_length, 1);
    assert_eq!(sapi_request_info.proto_num, 1100);
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.auth_user) }, c"user");
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(sapi_request_info.auth_password) }, c"password");

    sapi_request_info.free();
    free_raw_cstring_mut!(sapi_request_info, path_translated);
  }
}
