mod config;
mod response;
mod sapi;
mod service;
mod worker;

use crate::config::Config;
use crate::sapi::Sapi;
use crate::service::php::PhpService;
use crate::service::router::RouterLayer;
use crate::worker::start_php_worker_pool;
use anyhow::bail;
use clap::Parser;
use ext_php_rs::embed::{ext_php_rs_sapi_shutdown, ext_php_rs_sapi_startup};
use hyper::server::conn::http1::Builder;
use hyper_util::rt::TokioIo;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::request_id::MakeRequestUuid;
use tower_http::services::ServeDir;
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tower_http::ServiceBuilderExt;
use tracing::{error, info, Level};
use tracing_subscriber::EnvFilter;

async fn shutdown_signal() {
  tokio::signal::ctrl_c().await.expect("Failed to install CTRL+C handler");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let config = Config::parse();
  let listener = TcpListener::bind((config.address(), config.port())).await?;
  let php_pool = start_php_worker_pool(config.workers().unwrap_or(num_cpus::get_physical()))?;
  // the graceful watcher
  let graceful = GracefulShutdown::new();
  // when this signal completes, start shutdown
  let mut signal = std::pin::pin!(shutdown_signal());

  tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::from_default_env())
    .with_target(false)
    .init();

  unsafe {
    ext_php_rs_sapi_startup();
  }

  let sapi = Sapi::new();
  if sapi.startup().is_err() {
    bail!("Failed to start PHP SAPI module");
  };

  loop {
    tokio::select! {
      Ok((stream, _)) = listener.accept() => {
        let local_addr = stream.local_addr()?;
        let peer_addr = stream.peer_addr()?;

        let service = ServiceBuilder::new()
          .set_x_request_id(MakeRequestUuid)
          .layer(
            TraceLayer::new_for_http()
              .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
              .on_request(DefaultOnRequest::new().level(Level::INFO))
              .on_response(DefaultOnResponse::new().level(Level::INFO).include_headers(true)),
          )
          .propagate_x_request_id()
          .layer(AddExtensionLayer::new(Arc::new(config.root())))
          .layer(RouterLayer::new(PhpService::new(local_addr, peer_addr, php_pool.clone())))
          .service(ServeDir::new(config.root()));

        let connection = Builder::new().serve_connection(TokioIo::new(stream), TowerToHyperService::new(service));
        let future = graceful.watch(connection);
        tokio::spawn(async move {
          if let Err(err) = future.await {
            error!("Error serving connection: {:?}", err);
          }
        });
      },

      _ = &mut signal => {
        drop(listener);
        info!("graceful shutdown signal received");
        break;
      }
    }
  }

  tokio::select! {
    _ = graceful.shutdown() => {
      unsafe {
        sapi.shutdown();
        ext_php_rs_sapi_shutdown();
      }
      info!("all connections gracefully closed");
      Ok(())
    },
    _ = tokio::time::sleep(Duration::from_secs(10)) => {
      info!("timed out wait for all connections to close");
      Ok(())
    }
  }
}
