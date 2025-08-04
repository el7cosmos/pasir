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
