use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[cfg(not(php_zend_max_execution_timers))]
use ext_php_rs::zend::ExecutorGlobals;
use hyper::header::SERVER;
use hyper::http::HeaderValue;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::signal::unix::SignalKind;
use tower::ServiceBuilder;
#[cfg(not(php_zend_max_execution_timers))]
use tower::timeout::TimeoutLayer;
use tower_http::ServiceBuilderExt;
use tower_http::request_id::MakeRequestUuid;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::debug;
use tracing::error;
use tracing::info;

use crate::cli::Executable;
use crate::config::route::Routes;
use crate::service::PhpService;
use crate::service::RouterService;

#[derive(Debug)]
pub struct Stream {
  local_addr: SocketAddr,
  peer_addr: SocketAddr,
}

impl Stream {
  pub fn new(local_addr: SocketAddr, peer_addr: SocketAddr) -> Self {
    Self { local_addr, peer_addr }
  }

  pub fn local_addr(&self) -> SocketAddr {
    self.local_addr
  }

  pub fn peer_addr(&self) -> SocketAddr {
    self.peer_addr
  }
}

impl Default for Stream {
  fn default() -> Self {
    let socket = SocketAddr::new(IpAddr::from(Ipv4Addr::LOCALHOST), Default::default());
    Self { local_addr: socket, peer_addr: socket }
  }
}

#[derive(Clone, Debug)]
pub struct Serve {
  address: String,
  port: u16,
  root: PathBuf,
}

impl Serve {
  pub fn new(address: String, port: u16, root: PathBuf) -> Self {
    Self { address, port, root }
  }

  async fn serve(self) -> anyhow::Result<()> {
    info!("Pasir running on [http://{}:{}]", self.address, self.port);

    let routes = Arc::new(Routes::from_file(self.root.join("pasir.toml"))?);
    let listener = TcpListener::bind((self.address, self.port)).await?;
    let http = Builder::new(TokioExecutor::new());
    let graceful = GracefulShutdown::new();
    let mut sigterm = tokio::signal::unix::signal(SignalKind::terminate())?;
    let server = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

    loop {
      tokio::select! {
        Ok((stream, socket)) = listener.accept() => {
          let php_service = PhpService::default();
          let serve_dir = ServeDir::new(self.root.clone())
              .call_fallback_on_method_not_allowed(true)
              .append_index_html_on_directories(false)
              .precompressed_gzip();

          let tower_service = ServiceBuilder::new()
            .add_extension(Arc::new(self.root.clone()))
            .add_extension(routes.clone())
            .add_extension(Arc::new(Stream::new(stream.local_addr()?, socket)))
            .set_x_request_id(MakeRequestUuid)
            .layer(TraceLayer::new_for_http().on_request(()))
            .propagate_x_request_id()
            .insert_response_header_if_not_present(SERVER, HeaderValue::from_static(server));

          #[cfg(not(php_zend_max_execution_timers))]
          let tower_service = tower_service.map_result(crate::service::map_result)
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

        _ = tokio::signal::ctrl_c() => {
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
        info!("Gracefully shutdown");
        Ok(())
      },
      _ = tokio::time::sleep(Duration::from_secs(10)) => {
        info!("Time out while waiting for graceful shutdown, aborting");
        Ok(())
      }
    }
  }
}

impl Executable for Serve {
  async fn execute(self) -> anyhow::Result<()> {
    self.serve().await
  }
}
