use ext_php_rs::zend::SapiHeaders;
use hyper::StatusCode;
use hyper::http::status::InvalidStatusCode;

pub(crate) trait SapiHeadersExt {
  fn status(&self) -> Result<StatusCode, InvalidStatusCode>;
}

impl SapiHeadersExt for SapiHeaders {
  fn status(&self) -> Result<StatusCode, InvalidStatusCode> {
    match self.http_response_code.is_positive() {
      true => StatusCode::from_u16(self.http_response_code.cast_unsigned() as u16),
      false => Ok(StatusCode::default()),
    }
  }
}

#[cfg(test)]
mod tests {
  use crate::sapi::ext::SapiHeadersExt;
  use ext_php_rs::ffi::_zend_llist;
  use ext_php_rs::zend::SapiHeaders;
  use hyper::StatusCode;

  #[test]
  fn test_sapi_headers_status() {
    let headers = _zend_llist {
      head: std::ptr::null_mut(),
      tail: std::ptr::null_mut(),
      count: 0,
      size: 0,
      dtor: None,
      persistent: 0,
      traverse_ptr: std::ptr::null_mut(),
    };
    let mut sapi_headers = SapiHeaders {
      headers,
      http_response_code: 0,
      send_default_content_type: 0,
      mimetype: std::ptr::null_mut(),
      http_status_line: std::ptr::null_mut(),
    };
    assert!(sapi_headers.status().is_ok());
    assert_eq!(sapi_headers.status().unwrap(), StatusCode::default());

    sapi_headers.http_response_code = 1;
    assert!(sapi_headers.status().is_err());
    assert_eq!(format!("{}", sapi_headers.status().unwrap_err()), "invalid status code");

    sapi_headers.http_response_code = 2000;
    assert!(sapi_headers.status().is_err());
    assert_eq!(format!("{}", sapi_headers.status().unwrap_err()), "invalid status code");

    sapi_headers.http_response_code = 200;
    assert!(sapi_headers.status().is_ok());
    assert_eq!(sapi_headers.status().ok(), StatusCode::from_u16(200).ok());

    sapi_headers.http_response_code = 404;
    assert!(sapi_headers.status().is_ok());
    assert_eq!(sapi_headers.status().ok(), StatusCode::from_u16(404).ok());
  }
}
