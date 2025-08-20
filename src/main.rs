mod config;
mod sapi;
mod service;
mod unbound_channel;
mod util;

use crate::config::Config;
use crate::config::route::Routes;
use crate::sapi::Sapi;
use crate::service::php::PhpService;
use crate::service::router::RouterService;
use anyhow::bail;
use clap::Parser;
use ext_php_rs::embed::{ext_php_rs_sapi_shutdown, ext_php_rs_sapi_startup};
use hyper::header::SERVER;
use hyper::http::HeaderValue;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::signal::ctrl_c;
use tower::ServiceBuilder;
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
  let routes = Arc::new(Routes::from_file(config.root().join("pasir.toml"))?);
  let listener = TcpListener::bind((config.address(), config.port())).await?;
  let http = Builder::new(TokioExecutor::new());
  // the graceful watcher
  let graceful = GracefulShutdown::new();
  // when this signal completes, start shutdown
  let mut signal = std::pin::pin!(ctrl_c());

  unsafe { ext_php_rs_sapi_startup() }

  let sapi = Sapi::new();
  if sapi.startup().is_err() {
    bail!("Failed to start PHP SAPI module");
  };

  info!("Pasir running on [http://{}:{}]", config.address(), config.port());

  loop {
    tokio::select! {
      Ok((stream, socket)) = listener.accept() => {
        let server = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

        let php_service = PhpService::default();
        let serve_dir = ServeDir::new(config.root())
            .call_fallback_on_method_not_allowed(true)
            .append_index_html_on_directories(false)
            .precompressed_gzip();

        let service = ServiceBuilder::new()
          .add_extension(Arc::new(config.root()))
          .add_extension(routes.clone())
          .add_extension(Arc::new(Stream::new(stream.local_addr()?, socket)))
          .set_x_request_id(MakeRequestUuid)
          .layer(TraceLayer::new_for_http().on_request(()))
          .propagate_x_request_id()
          .insert_response_header_if_not_present(SERVER, HeaderValue::from_static(server))
          .service(RouterService::new(serve_dir, php_service));

        let connection = http.serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(service));
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

      _ = signal.as_mut() => {
        drop(listener);
        info!("Starting graceful shutdown");
        break;
      }
    }
  }

  tokio::select! {
    _ = graceful.shutdown() => {
      sapi.shutdown();
      unsafe { ext_php_rs_sapi_shutdown() }
      info!("Gracefully shutdown");
      Ok(())
    },
    _ = tokio::time::sleep(Duration::from_secs(10)) => {
      sapi.shutdown();
      unsafe { ext_php_rs_sapi_shutdown() }
      info!("Time out while waiting for graceful shutdown, aborting");
      Ok(())
    }
  }
}
