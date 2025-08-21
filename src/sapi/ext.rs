use ext_php_rs::zend::SapiHeaders;
use hyper::StatusCode;
use hyper::http::status::InvalidStatusCode;

pub(crate) trait FromSapiHeaders: Sized {
  type Err;

  fn from_sapi_headers(headers: &SapiHeaders) -> Result<Self, Self::Err>;
}

impl FromSapiHeaders for StatusCode {
  type Err = InvalidStatusCode;

  fn from_sapi_headers(headers: &SapiHeaders) -> Result<Self, Self::Err> {
    if headers.http_response_code == 0 {
      return Ok(Self::default());
    }

    if let Ok(rc) = u16::try_from(headers.http_response_code) {
      return Self::from_u16(rc);
    }

    let bytes = headers.http_response_code.to_ne_bytes();
    Self::from_bytes(&bytes)
  }
}

#[cfg(test)]
mod tests {
  use crate::sapi::ext::FromSapiHeaders;
  use ext_php_rs::ffi::_zend_llist;
  use ext_php_rs::zend::SapiHeaders;
  use hyper::StatusCode;
  use proptest::prelude::*;
  use std::os::raw::c_int;

  trait SapiHeadersTestExt {
    fn new(rc: c_int) -> SapiHeaders;
  }

  impl SapiHeadersTestExt for SapiHeaders {
    fn new(rc: c_int) -> SapiHeaders {
      let headers = _zend_llist {
        head: std::ptr::null_mut(),
        tail: std::ptr::null_mut(),
        count: 0,
        size: 0,
        dtor: None,
        persistent: 0,
        traverse_ptr: std::ptr::null_mut(),
      };
      SapiHeaders {
        headers,
        http_response_code: rc,
        send_default_content_type: 0,
        mimetype: std::ptr::null_mut(),
        http_status_line: std::ptr::null_mut(),
      }
    }
  }

  #[test]
  fn sapi_headers_default() {
    let sapi_headers = SapiHeaders::new(0);
    let status = StatusCode::from_sapi_headers(&sapi_headers);
    assert!(status.is_ok());
    assert_eq!(status.unwrap(), StatusCode::default());
  }

  proptest! {
    #[test]
    fn sapi_headers_valid_code(rc in 100..1000u16) {
      let sapi_headers = SapiHeaders::new(c_int::from(rc));
      let status = StatusCode::from_sapi_headers(&sapi_headers);
      assert!(status.is_ok());
      assert_eq!(status.ok(), StatusCode::from_u16(rc).ok());
    }

    #[test]
    fn sapi_headers_invalid_code(rc in prop_oneof![..100, 1001..]) {
      let sapi_headers = SapiHeaders::new(rc);
      assert!(StatusCode::from_sapi_headers(&sapi_headers).is_err());
    }
  }
}
