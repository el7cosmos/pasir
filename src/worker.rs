use crate::sapi::context::Context;
use anyhow::Error;
use bytes::Bytes;
use ext_php_rs::embed::{ext_php_rs_sapi_per_thread_init, Embed};
use ext_php_rs::ffi::{php_request_shutdown, php_request_startup, ZEND_RESULT_CODE_FAILURE};
use ext_php_rs::zend::{try_catch_first, SapiGlobals};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{Response, StatusCode};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;
use tracing::{debug, error};

pub(crate) fn start_php_worker_pool(size: usize) -> anyhow::Result<mpsc::Sender<Context>> {
  let (tx, rx) = mpsc::channel::<Context>(size * 10);
  let shared_rx = Arc::new(Mutex::new(rx));

  for worker in 0..size {
    let rx_clone = Arc::clone(&shared_rx);
    thread::spawn(move || {
      loop {
        let maybe_job = {
          let mut rx_lock = rx_clone.lock().unwrap();
          rx_lock.blocking_recv()
        };

        match maybe_job {
          Some(context) => {
            debug!("Serving php from worker {}", worker);
            unsafe {
              ext_php_rs_sapi_per_thread_init();
            }

            if context.init_globals().is_err() {
              error!("context.init_globals failed");
              break;
            }

            let script = context.root().join(context.script_name().trim_start_matches("/"));

            let context_raw = Box::into_raw(Box::new(context));
            SapiGlobals::get_mut().server_context = context_raw.cast::<c_void>();

            if unsafe { php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
              error!("php_request_startup failed");
              break;
            }

            let _tried = try_catch_first(|| {
              let _script = Embed::run_script(script.clone());
            });

            unsafe {
              php_request_shutdown(std::ptr::null_mut());
            }

            let sapi_globals = SapiGlobals::get();
            let sapi_headers = sapi_globals.sapi_headers();
            if let Some(context) = Context::from_server_context(sapi_globals.server_context) {
              let mut response = Response::new(UnsyncBoxBody::new(
                Full::new(Bytes::from(context.buffer.to_owned())).map_err(Error::from),
              ));

              if sapi_headers.http_response_code.is_positive() {
                *response.status_mut() =
                  StatusCode::from_u16(sapi_headers.http_response_code.cast_unsigned() as u16)
                    .unwrap_or_default();
              }

              *response.headers_mut() = context.response_head.clone();

              if context.send_response(response).is_err() {
                error!("send response failed");
              };
            }
          }
          None => {
            break;
          }
        }
      }
    });
  }

  Ok(tx)
}
