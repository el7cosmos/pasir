use ext_php_rs::zend::SapiHeaders;
use hyper::StatusCode;

pub(crate) trait SapiHeadersExt {
  fn status(&self) -> StatusCode;
}

impl SapiHeadersExt for SapiHeaders {
  fn status(&self) -> StatusCode {
    match self.http_response_code.is_positive() {
      true => {
        StatusCode::from_u16(self.http_response_code.cast_unsigned() as u16).unwrap_or_default()
      }
      false => StatusCode::default(),
    }
  }
}
