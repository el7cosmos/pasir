use hyper::{Response, StatusCode};
use std::convert::Infallible;

pub(crate) trait ResponseExt<T> {
  fn bad_request(body: T) -> Result<Response<T>, Infallible>;
  fn internal_server_error(body: T) -> Result<Response<T>, Infallible>;
  fn service_unavailable(body: T) -> Result<Response<T>, Infallible>;
  fn gateway_timeout(body: T) -> Result<Response<T>, Infallible>;
}

impl<T> ResponseExt<T> for Response<T> {
  fn bad_request(body: T) -> Result<Response<T>, Infallible> {
    Ok(make_response(StatusCode::BAD_REQUEST, body))
  }

  fn internal_server_error(body: T) -> Result<Self, Infallible> {
    Ok(make_response(StatusCode::INTERNAL_SERVER_ERROR, body))
  }

  fn service_unavailable(body: T) -> Result<Response<T>, Infallible> {
    Ok(make_response(StatusCode::SERVICE_UNAVAILABLE, body))
  }

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
  use crate::util::response_ext::ResponseExt;
  use hyper::{Response, StatusCode};
  use rstest::rstest;
  use std::convert::Infallible;

  #[rstest]
  #[case::bad_request(Response::bad_request, StatusCode::BAD_REQUEST)]
  #[case::internal_server_error(Response::internal_server_error, StatusCode::INTERNAL_SERVER_ERROR)]
  #[case::service_unavailable(Response::service_unavailable, StatusCode::SERVICE_UNAVAILABLE)]
  #[case::gateway_timeout(Response::gateway_timeout, StatusCode::GATEWAY_TIMEOUT)]
  fn test_response_ext<F: Fn(String) -> Result<Response<String>, Infallible>>(
    #[case] f: F,
    #[case] status: StatusCode,
  ) {
    let response = f("Foo".to_string());
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), status);
  }
}
