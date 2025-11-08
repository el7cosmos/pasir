use std::convert::Infallible;
use std::ffi::CString;
use std::ffi::c_void;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use bytes::Bytes;
use ext_php_rs::embed::Embed;
use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::try_catch_first;
use http_body_util::BodyExt;
use http_body_util::Empty;
use http_body_util::Full;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::Request;
use hyper::Response;
use hyper::body::Body;
use pasir::error::PhpError;
use pasir::ffi::ZEND_RESULT_CODE_FAILURE;
use tower::Service;
use tracing::error;
use tracing::instrument;

use crate::cli::serve::Stream;
use crate::free_raw_cstring_mut;
use crate::sapi::context::Context;
use crate::sapi::context::ContextSender;
use crate::sapi::context::ResponseType;
use crate::util::response_ext::ResponseExt;

#[derive(Clone, Default)]
pub(crate) struct PhpService {}

impl<B> Service<Request<B>> for PhpService
where
  B: Body + Send + 'static,
  B::Data: Send,
{
  type Response = Response<UnsyncBoxBody<Bytes, Infallible>>;
  type Error = Infallible;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, req: Request<B>) -> Self::Future {
    let root = req.extensions().get::<Arc<PathBuf>>().unwrap().clone();
    let stream = req.extensions().get::<Arc<Stream>>().unwrap().clone();
    let error_body = Empty::default().boxed_unsync();

    Box::pin(async move {
      let (head, body) = req.into_parts();
      let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Response::internal_server_error(error_body),
      };

      let (error_tx, error_rx) =
        tokio::sync::oneshot::channel::<fn(error_body: UnsyncBoxBody<Bytes, Infallible>) -> Result<Self::Response, Infallible>>();
      let (head_rx, body_rx, context_tx) = ContextSender::receiver();

      tokio::task::spawn_blocking(move || {
        unsafe { ext_php_rs::embed::ext_php_rs_sapi_per_thread_init() }
        unsafe { pasir::ffi::zend_update_current_locale() }

        let request = Request::from_parts(head, bytes);
        let context = Context::new(root, stream, request, context_tx);
        if context.init_sapi_globals().is_err() {
          return error_tx.send(Response::bad_request);
        }

        if let Err(e) = execute_php(context) {
          let callback = match e {
            PhpError::RequestStartupFailed => Response::service_unavailable,
            PhpError::ServerContextCorrupted => Response::internal_server_error,
          };
          return error_tx.send(callback);
        }

        Ok(())
      });

      tokio::select! {
        Ok(callback) = error_rx => {
          callback(error_body)
        }
        Ok(mut head) = head_rx => {
          let response_type = head.extensions.get_or_insert_default::<ResponseType>();
          let body = match response_type {
            ResponseType::Full => Full::new(body_rx.collect().await.unwrap().to_bytes()).boxed_unsync(),
            ResponseType::Chunked => body_rx.boxed_unsync(),
          };
          let response = Response::from_parts(head, body);
          Ok(response)
        }
        else => Response::internal_server_error(error_body)
      }
    })
  }
}

#[instrument(skip(context), err)]
fn execute_php(context: Context) -> Result<(), PhpError> {
  let script = context.root().join(context.script_name().trim_start_matches("/"));

  let context_raw = context.into_raw();
  SapiGlobals::get_mut().server_context = context_raw.cast::<c_void>();

  if unsafe { pasir::ffi::php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
    return Err(PhpError::RequestStartupFailed);
  }

  let catch = try_catch_first(|| {
    if let Err(e) = Embed::run_script(script.as_path())
      && e.is_bailout()
    {
      error!("run_script failed: {:?}", e);
    }
  });

  request_shutdown();

  if let Err(e) = catch {
    error!("catch failed: {:?}", e);
  }

  Ok(())
}

fn request_shutdown() {
  #[cfg(php84)]
  unsafe {
    pasir::ffi::zend_shutdown_strtod()
  };
  unsafe { pasir::ffi::php_request_shutdown(std::ptr::null_mut()) };

  let mut request_info = SapiGlobals::get().request_info;
  free_raw_cstring_mut!(request_info, path_translated);
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::sync::Arc;

  use bytes::Bytes;
  use http_body_util::Empty;
  use hyper::Request;
  use hyper::StatusCode;
  use hyper::body::Body;
  use tower::Service;

  use crate::cli::serve::Stream;
  use crate::sapi::Sapi;
  use crate::service::PhpService;

  #[tokio::test]
  async fn test_php_service() {
    let sapi = Sapi::new(false, None);
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() }
    assert!(sapi.startup().is_ok());

    let root = PathBuf::from("tests/fixtures/root").canonicalize().unwrap();
    let stream = Stream::default();
    let request = Request::builder()
      .extension(Arc::new(root))
      .extension(Arc::new(stream))
      .body(Empty::<Bytes>::default())
      .unwrap();

    let mut service = PhpService::default();

    let response = service.call(request.clone()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_ne!(response.body().size_hint().lower(), 0);

    // Assert that request shutdown cleanly and further requests can return a response.
    let response = service.call(request.clone()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_ne!(response.body().size_hint().lower(), 0);
  }
}
