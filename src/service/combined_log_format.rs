use crate::util::request_ext::RequestExt;
use chrono::Utc;
use hyper::body::Incoming;
use hyper::{Request, Response};
use std::net::IpAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::Service;

#[derive(Clone)]
pub(crate) struct CombinedLogFormat<S> {
  inner: S,
}

impl<S> CombinedLogFormat<S> {
  pub(crate) fn new(inner: S) -> Self {
    Self { inner }
  }
}

impl<S, ResBody> Service<Request<Incoming>> for CombinedLogFormat<S>
where
  S: Service<Request<Incoming>, Response = Response<ResBody>> + Clone,
  S::Future: Send + 'static,
{
  type Response = S::Response;
  type Error = S::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    self.inner.poll_ready(cx)
  }

  fn call(&mut self, req: Request<Incoming>) -> Self::Future {
    let client_ip =
      req.client_ip().map(|ip_addr: IpAddr| ip_addr.to_string()).unwrap_or("unknown".to_string());
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let user_agent =
      req.headers().get("user-agent").and_then(|h| h.to_str().ok()).unwrap_or("-").to_string();
    let referer =
      req.headers().get("referer").and_then(|h| h.to_str().ok()).unwrap_or("-").to_string();

    let future = self.inner.call(req);
    Box::pin(async move {
      let response = future.await?;

      // Log in Apache Combined Log Format
      let datetime = Utc::now();
      let timestamp = datetime.format("[%d/%b/%Y:%H:%M:%S %z]");

      let status = response.status().as_u16();

      // Print Apache-style access log
      println!(
        r#"{client_ip} - - {timestamp} "{method} {uri} HTTP/1.1" {status} - "{referer}" "{user_agent}""#
      );

      Ok(response)
    })
  }
}
