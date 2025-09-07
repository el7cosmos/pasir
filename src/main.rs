mod config;
mod sapi;
mod service;
mod util;

use crate::config::Config;
use crate::config::route::Routes;
use crate::sapi::Sapi;
use crate::service::PhpService;
use crate::service::RouterService;
use anyhow::bail;
use clap::Parser;
use ext_php_rs::ffi::{
  ZEND_RESULT_CODE_FAILURE, module_registry, zend_extension, zend_extensions, zend_module_entry,
};
#[cfg(not(php_zend_max_execution_timers))]
use ext_php_rs::zend::ExecutorGlobals;
use hyper::header::SERVER;
use hyper::http::HeaderValue;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use nu_ansi_term::Color;
use pasir::error::PhpError;
use std::borrow::Cow;
use std::ffi::CStr;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
use tokio::signal::unix::signal;
use tower::ServiceBuilder;
#[cfg(not(php_zend_max_execution_timers))]
use tower::timeout::TimeoutLayer;
use tower_http::ServiceBuilderExt;
use tower_http::request_id::MakeRequestUuid;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info};
use tracing_subscriber::fmt::format;

#[derive(Debug)]
struct Stream {
  local_addr: SocketAddr,
  peer_addr: SocketAddr,
}

impl Stream {
  fn new(local_addr: SocketAddr, peer_addr: SocketAddr) -> Self {
    Self { local_addr, peer_addr }
  }
}

#[tokio::main]
async fn main() {
  let config = Config::parse();

  let format = format().compact();
  tracing_subscriber::fmt()
    .event_format(format)
    .with_max_level(config.verbosity())
    .with_target(false)
    .init();

  let result = start(config).await;
  if let Err(err) = result {
    error!("{}", err);
    std::process::exit(1);
  };
}

async fn start(config: Config) -> anyhow::Result<()> {
  unsafe { ext_php_rs::embed::ext_php_rs_sapi_startup() }

  let sapi = Sapi::new();
  if sapi.startup().is_err() {
    bail!("Failed to start PHP SAPI module");
  };

  if config.is_module() {
    return match unsafe { print_modules() } {
      Ok(_) => {
        shutdown(sapi);
        Ok(())
      }
      Err(e) => {
        shutdown(sapi);
        Err(e.into())
      }
    };
  }

  let address = config.address();
  let port = config.port().unwrap();

  info!("Pasir running on [http://{}:{}]", address, port);

  let routes = Arc::new(Routes::from_file(config.root().join("pasir.toml"))?);
  let listener = TcpListener::bind((address, port)).await?;
  let http = Builder::new(TokioExecutor::new());
  let graceful = GracefulShutdown::new();
  let mut sigterm = signal(tokio::signal::unix::SignalKind::terminate())?;
  let server = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

  loop {
    tokio::select! {
      Ok((stream, socket)) = listener.accept() => {
        let php_service = PhpService::default();
        let serve_dir = ServeDir::new(config.root())
            .call_fallback_on_method_not_allowed(true)
            .append_index_html_on_directories(false)
            .precompressed_gzip();

        let tower_service = ServiceBuilder::new()
          .add_extension(Arc::new(config.root()))
          .add_extension(routes.clone())
          .add_extension(Arc::new(Stream::new(stream.local_addr()?, socket)))
          .set_x_request_id(MakeRequestUuid)
          .layer(TraceLayer::new_for_http().on_request(()))
          .propagate_x_request_id()
          .insert_response_header_if_not_present(SERVER, HeaderValue::from_static(server));

        #[cfg(not(php_zend_max_execution_timers))]
        let tower_service = tower_service.map_result(service::map_result)
          .layer(TimeoutLayer::new(Duration::from_secs(ExecutorGlobals::get().timeout_seconds.cast_unsigned())));

        let tower_service = tower_service.service(RouterService::new(serve_dir, php_service));

        let connection = http.serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(tower_service));
        let future = graceful.watch(connection.into_owned());
        tokio::spawn(async move {
          if let Err(err) = future.await {
            if let Some(hyper_error) = err.downcast_ref::<hyper::Error>() && hyper_error.is_incomplete_message() {
              debug!("Error serving connection: {err}");
            }
            else {
              error!("Error serving connection: {err}");
            }
          }
        });
      },

      _ = ctrl_c() => {
        drop(listener);
        info!("Starting graceful shutdown");
        break;
      }
      _ = sigterm.recv() => {
        drop(listener);
        info!("Starting graceful shutdown");
        break;
      }
    }
  }

  tokio::select! {
    _ = graceful.shutdown() => {
      shutdown(sapi);
      info!("Gracefully shutdown");
      Ok(())
    },
    _ = tokio::time::sleep(Duration::from_secs(10)) => {
      shutdown(sapi);
      info!("Time out while waiting for graceful shutdown, aborting");
      Ok(())
    }
  }
}

unsafe fn print_modules() -> anyhow::Result<(), PhpError> {
  if unsafe { ext_php_rs::ffi::php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
    return Err(PhpError::RequestStartupFailed);
  }

  let stdout = std::io::stdout();
  let mut handle = std::io::BufWriter::new(stdout);

  writeln!(handle, "{}", Color::Cyan.bold().paint("[PHP Modules]")).unwrap();

  let registry_ptr = std::ptr::addr_of_mut!(module_registry);
  let mut modules = unsafe { (*registry_ptr).values() }
    .map(|value| {
      let entry = unsafe { value.ptr::<zend_module_entry>().unwrap() };
      let name = unsafe { CStr::from_ptr((*entry).name) };
      name.to_string_lossy()
    })
    .collect::<Vec<Cow<str>>>();
  modules.sort_by_key(|key| key.to_ascii_lowercase());
  for value in modules {
    writeln!(handle, "{}", value).unwrap();
  }

  writeln!(handle, "\n{}", Color::Cyan.bold().paint("[Zend Modules]")).unwrap();

  let extensions_ptr = std::ptr::addr_of_mut!(zend_extensions);
  let mut extensions = unsafe { (*extensions_ptr).iter() }
    .map(|zend_extension: &zend_extension| {
      let name = unsafe { CStr::from_ptr(zend_extension.name) };
      name.to_string_lossy()
    })
    .collect::<Vec<Cow<str>>>();
  extensions.sort_by_key(|key| key.to_ascii_lowercase());
  for extension in extensions {
    writeln!(handle, "{}", extension).unwrap();
  }

  Ok(())
}

fn shutdown(sapi: Sapi) {
  sapi.shutdown();
  unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() }
}
