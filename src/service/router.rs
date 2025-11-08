use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use http_body_util::BodyExt;
use hyper::Request;
use hyper::Response;
use hyper::body::Body;
use tower::Service;
use tower_http::services::ServeDir;
use tower_http::services::fs::ServeFileSystemResponseBody;

use crate::config::route::ApplyActions;
use crate::config::route::RouteServe;
use crate::config::route::Routes;
use crate::service::ResponseBody;
use crate::service::php::PhpService;

#[derive(Clone)]
pub(crate) struct RouterService {
  inner: ServeDir,
  php: PhpService,
}

impl RouterService {
  pub(crate) fn new(inner: ServeDir, php: PhpService) -> Self {
    Self { inner, php }
  }

  fn fallback(&self) -> ServeDir<PhpService> {
    self.inner.clone().fallback(self.php.clone())
  }

  fn map_serve_dir_response(response: Response<ServeFileSystemResponseBody>) -> Response<ResponseBody> {
    response.map(|body| body.map_err(|_| unreachable!()).boxed_unsync())
  }
}

impl<B> Service<Request<B>> for RouterService
where
  B: Body + Send + 'static,
  B::Data: Send,
{
  type Response = Response<ResponseBody>;
  type Error = Infallible;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
    <PhpService as Service<Request<B>>>::poll_ready(&mut self.php, cx)
  }

  fn call(&mut self, req: Request<B>) -> Self::Future {
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
