use crate::sapi::context::Context;
use ext_php_rs::embed::{Embed, ext_php_rs_sapi_per_thread_init};
use ext_php_rs::ffi::{ZEND_RESULT_CODE_FAILURE, php_request_shutdown, php_request_startup};
use ext_php_rs::zend::{SapiGlobals, try_catch_first};
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
        let context_rx = {
          let mut rx_lock = rx_clone.lock().unwrap();
          rx_lock.blocking_recv()
        };

        match context_rx {
          Some(context) => {
            debug!("Serving php from worker {}", worker);
            unsafe {
              ext_php_rs_sapi_per_thread_init();
            }

            if context.init_globals().is_err() {
              error!("context.init_globals failed");
              break;
            }

            let script = context.root().join(context.route().script_name().trim_start_matches("/"));

            let context_raw = Box::into_raw(Box::new(context));
            SapiGlobals::get_mut().server_context = context_raw.cast::<c_void>();

            if unsafe { php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
              error!("php_request_startup failed");
              break;
            }

            let _tried = try_catch_first(|| {
              let _script = Embed::run_script(script.as_path());
            });

            unsafe {
              php_request_shutdown(std::ptr::null_mut());
            }

            if let Some(context) = Context::from_server_context(SapiGlobals::get().server_context) {
              if !context.is_request_finished() && !context.finish_request() {
                error!("finish request failed");
              }
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
