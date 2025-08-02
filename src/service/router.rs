use crate::service::php::PhpService;
use anyhow::{Error, anyhow};
use bytes::BytesMut;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;
use hyper::{Request, Response};
use regex::Regex;
use std::pin::Pin;
use std::task::Poll;
use tower::{Layer, Service};
use tower_http::services::ServeDir;

#[derive(Clone)]
pub(crate) struct RouterService {
  inner: ServeDir,
  php: PhpService,
}

impl RouterService {
  pub(crate) fn new(inner: ServeDir, php: PhpService) -> Self {
    Self { inner, php }
  }
}

impl Service<Request<Incoming>> for RouterService {
  type Response = Response<UnsyncBoxBody<BytesMut, Error>>;
  type Error = Error;
  type Future = Pin<Box<dyn Future<Output = anyhow::Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, request: Request<Incoming>) -> Self::Future {
    let uri = request.uri();
    if uri.path() == "/" || uri.path().ends_with(".php") {
      return self.php.call(request);
    }

    let regex = Regex::new(r"\.(js|css|png|jpg|jpeg|gif|ico|svg|woff2)$").unwrap();
    if !regex.is_match(uri.path()) {
      return self.php.call(request);
    }

    let future = self.inner.call(request);
    Box::pin(async move {
      match future.await {
        Ok(response) => {
          let (head, body) = response.into_parts();
          Ok(Response::from_parts(
            head,
            body
              .map_frame(|frame| frame.map_data(BytesMut::from))
              .map_err(|error| anyhow!(error.to_string()))
              .boxed_unsync(),
          ))
        }
        Err(error) => Err(anyhow!(error.to_string())),
      }
    })
  }
}

pub(crate) struct RouterLayer {
  php_service: PhpService,
}

impl RouterLayer {
  pub(crate) fn new(php_service: PhpService) -> Self {
    Self { php_service }
  }
}

impl Layer<ServeDir> for RouterLayer {
  type Service = RouterService;

  fn layer(&self, inner: ServeDir) -> Self::Service {
    RouterService::new(inner, self.php_service.clone())
  }
}
