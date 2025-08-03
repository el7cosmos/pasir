use hyper::{Response, StatusCode};
use std::convert::Infallible;

pub(crate) trait InternalServerError<T> {
  fn internal_server_error(body: T) -> Result<Response<T>, Infallible>;
}

impl<T> InternalServerError<T> for Response<T> {
  fn internal_server_error(body: T) -> Result<Self, Infallible>
  where
    Self: Sized,
  {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    Ok(response)
  }
}
