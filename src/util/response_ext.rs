use std::convert::Infallible;

use hyper::Response;
use hyper::StatusCode;

pub(crate) trait ResponseExt<T> {
  fn bad_request(body: T) -> Result<Response<T>, Infallible>;
  fn internal_server_error(body: T) -> Result<Response<T>, Infallible>;
  fn service_unavailable(body: T) -> Result<Response<T>, Infallible>;
  #[cfg(not(php_zend_max_execution_timers))]
  fn gateway_timeout(body: T) -> Result<Response<T>, Infallible>;
}

impl<T> ResponseExt<T> for Response<T> {
  fn bad_request(body: T) -> Result<Self, Infallible> {
    Ok(make_response(StatusCode::BAD_REQUEST, body))
  }

  fn internal_server_error(body: T) -> Result<Self, Infallible> {
    Ok(make_response(StatusCode::INTERNAL_SERVER_ERROR, body))
  }

  fn service_unavailable(body: T) -> Result<Self, Infallible> {
    Ok(make_response(StatusCode::SERVICE_UNAVAILABLE, body))
  }

  #[cfg(not(php_zend_max_execution_timers))]
  fn gateway_timeout(body: T) -> Result<Self, Infallible> {
    Ok(make_response(StatusCode::GATEWAY_TIMEOUT, body))
  }
}

fn make_response<T>(status: StatusCode, body: T) -> Response<T> {
  let mut response = Response::new(body);
  *response.status_mut() = status;
  response
}

#[cfg(test)]
mod tests {
  use std::convert::Infallible;

  use hyper::Response;
  use hyper::StatusCode;
  use rstest::rstest;

  use crate::util::response_ext::ResponseExt;

  #[rstest]
  #[case::bad_request(Response::bad_request, StatusCode::BAD_REQUEST)]
  #[case::internal_server_error(Response::internal_server_error, StatusCode::INTERNAL_SERVER_ERROR)]
  #[case::service_unavailable(Response::service_unavailable, StatusCode::SERVICE_UNAVAILABLE)]
  fn test_response_ext<F: Fn(String) -> Result<Response<String>, Infallible>>(#[case] f: F, #[case] status: StatusCode) {
    let response = f("Foo".to_string());
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), status);
  }

  #[cfg(not(php_zend_max_execution_timers))]
  #[test]
  fn test_response_ext_gateway_timeout() {
    let response = Response::gateway_timeout("Foo".to_string());
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
  }
}
