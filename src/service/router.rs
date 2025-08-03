use crate::service::serve_php::ServePhp;
use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;
use hyper::{Request, Response};
use regex::Regex;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::Poll;
use tower::{Layer, Service};
use tower_http::services::ServeDir;

#[derive(Clone)]
pub(crate) struct RouterService {
  inner: ServeDir<ServePhp>,
  php: ServePhp,
}

impl RouterService {
  pub(crate) fn new(inner: ServeDir<ServePhp>, php: ServePhp) -> Self {
    Self { inner, php }
  }
}

impl Service<Request<Incoming>> for RouterService {
  type Response = Response<UnsyncBoxBody<Bytes, Infallible>>;
  type Error = Infallible;
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
      future
        .await
        .map(|response| response.map(|body| body.map_err(|_| unreachable!()).boxed_unsync()))
    })
  }
}

pub(crate) struct RouterLayer {
  php_service: ServePhp,
}

impl RouterLayer {
  pub(crate) fn new(php_service: ServePhp) -> Self {
    Self { php_service }
  }
}

impl Layer<ServeDir<ServePhp>> for RouterLayer {
  type Service = RouterService;

  fn layer(&self, inner: ServeDir<ServePhp>) -> Self::Service {
    RouterService::new(inner, self.php_service.clone())
  }
}
