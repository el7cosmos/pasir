use std::convert::Infallible;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Empty;
use http_body_util::Full;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::Request;
use hyper::Response;
use hyper::body::Body;
use pasir_sapi::context::ServerContext;
use pasir_sapi::error::ExecutePhpError;
use tower::Service;
use tracing::error;

use crate::cli::serve::Stream;
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
        unsafe { pasir_sys::zend_update_current_locale() }

        let request = Request::from_parts(head, bytes);
        let context = Context::new(root.clone(), stream, request, context_tx);
        let script = root.join(context.script_name().trim_start_matches("/"));
        if let Err(e) = context.execute_php(script, |err| {
          error!("run_script failed: {:?}", err);
        }) {
          let callback = match e {
            ExecutePhpError::InitSapiGlobalsError(_) => Response::bad_request,
            ExecutePhpError::RequestStartupFailed => Response::service_unavailable,
            ExecutePhpError::Bailout => Response::internal_server_error,
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

#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::sync::Arc;

  use bytes::Bytes;
  use http_body_util::Empty;
  use hyper::Request;
  use hyper::StatusCode;
  use hyper::body::Body;
  use pasir_sapi::Sapi as PasirSapi;
  use pasir_sys::ZEND_RESULT_CODE_SUCCESS;
  use tower::Service;

  use crate::cli::serve::Stream;
  use crate::sapi::Sapi;
  use crate::service::PhpService;

  #[tokio::test]
  async fn test_php_service() {
    let sapi = Sapi::new(false, None);
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() }
    assert_eq!(sapi.sapi_startup(), ZEND_RESULT_CODE_SUCCESS);

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
