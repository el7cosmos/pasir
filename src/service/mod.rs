use std::convert::Infallible;

use bytes::Bytes;
#[cfg(not(php_zend_max_execution_timers))]
use http_body_util::BodyExt;
#[cfg(not(php_zend_max_execution_timers))]
use http_body_util::Empty;
use http_body_util::combinators::UnsyncBoxBody;
#[cfg(not(php_zend_max_execution_timers))]
use hyper::Response;
#[cfg(not(php_zend_max_execution_timers))]
use tower::BoxError;
#[cfg(not(php_zend_max_execution_timers))]
use tower::timeout::error::Elapsed;

#[cfg(not(php_zend_max_execution_timers))]
use crate::util::response_ext::ResponseExt;

pub(crate) mod php;
mod router;

pub(crate) use php::PhpService;
pub(crate) use router::RouterService;

type ResponseBody = UnsyncBoxBody<Bytes, Infallible>;
#[cfg(not(php_zend_max_execution_timers))]
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
