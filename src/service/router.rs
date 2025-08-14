use crate::config::route::{ApplyActions, RouteServe, Routes};
use crate::service::serve_php::ServePhp;
use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;
use hyper::{Request, Response};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use tower::Service;
use tower_http::services::ServeDir;
use tower_http::services::fs::ServeFileSystemResponseBody;

type ResponseBody = UnsyncBoxBody<Bytes, Infallible>;

#[derive(Clone)]
pub(crate) struct RouterService {
  inner: ServeDir,
  php: ServePhp,
}

impl RouterService {
  pub(crate) fn new(inner: ServeDir, php: ServePhp) -> Self {
    Self { inner, php }
  }

  fn fallback(&self) -> ServeDir<ServePhp> {
    self.inner.clone().fallback(self.php.clone())
  }

  fn map_serve_dir_response(
    response: Response<ServeFileSystemResponseBody>,
  ) -> Response<ResponseBody> {
    response.map(|body| body.map_err(|_| unreachable!()).boxed_unsync())
  }
}

impl Service<Request<Incoming>> for RouterService {
  type Response = Response<ResponseBody>;
  type Error = Infallible;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
    self.php.poll_ready(_cx)
  }

  fn call(&mut self, req: Request<Incoming>) -> Self::Future {
    let routes = req.extensions().get::<Arc<Routes>>().unwrap().clone();
    if let Some(mut served_route) = routes.served_route(&req) {
      let future = match served_route.serve() {
        RouteServe::Php => self.php.call(req),
        RouteServe::Default => Box::pin(async move { Ok(Response::default()) }),
        RouteServe::Static => {
          let future = self.inner.call(req);
          Box::pin(async move { future.await.map(Self::map_serve_dir_response) })
        }
      };

      return Box::pin(async move {
        future.await.map(|mut response| {
          served_route.apply_actions(&mut response);
          response
        })
      });
    }

    let path = req.uri().path();
    let future = match path.ends_with("/") || path.ends_with(".php") {
      true => self.php.call(req),
      false => {
        let future = self.fallback().call(req);
        Box::pin(async move { future.await.map(Self::map_serve_dir_response) })
      }
    };

    Box::pin(async move {
      future.await.map(|mut response| {
        routes.apply_actions(&mut response);
        response
      })
    })
  }
}
