use crate::util::response_ext::ResponseExt;
use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::Response;
use std::convert::Infallible;
use tower::BoxError;
use tower::timeout::error::Elapsed;

pub(crate) mod php;
mod router;

pub(crate) use php::PhpService;
pub(crate) use router::RouterService;

type ResponseBody = UnsyncBoxBody<Bytes, Infallible>;
type MapResult = Result<Response<ResponseBody>, BoxError>;

#[cfg(not(php_zend_max_execution_timers))]
pub(crate) fn map_result(result: MapResult) -> MapResult {
  result.or_else(|err| {
    if err.is::<Elapsed>() {
      return Ok(Response::gateway_timeout(Empty::default().boxed_unsync())?);
    }

    Err(err)
  })
}
