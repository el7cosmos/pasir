use hyper::{Response, StatusCode};
use std::convert::Infallible;

pub(crate) trait ResponseExt<T> {
  fn internal_server_error(body: T) -> Result<Response<T>, Infallible>;
}

impl<T> ResponseExt<T> for Response<T> {
  fn internal_server_error(body: T) -> Result<Self, Infallible> {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    Ok(response)
  }
}

#[cfg(test)]
mod tests {
  use crate::util::response_ext::ResponseExt;
  use hyper::{Response, StatusCode};

  #[test]
  fn internal_server_error() {
    let response = Response::internal_server_error("Foo");
    assert!(response.is_ok());
    assert_eq!(response.unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
  }
}
