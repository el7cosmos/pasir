use hyper::{Response, StatusCode};

pub(crate) trait InternalServerError<T> {
  fn internal_server_error(body: T) -> anyhow::Result<Response<T>>;
}

impl<T> InternalServerError<T> for Response<T> {
  fn internal_server_error(body: T) -> anyhow::Result<Self>
  where
    Self: Sized,
  {
    Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(body).map_err(Into::into)
  }
}
