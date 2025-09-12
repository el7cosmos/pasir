mod cli;
mod config;
mod sapi;
mod service;
mod util;

use crate::cli::Cli;
use crate::cli::Executable;
use clap::Parser;
use tracing::error;

#[tokio::main]
async fn main() {
  let cli = Cli::parse();

  let format = tracing_subscriber::fmt::format().compact();
  tracing_subscriber::fmt()
    .event_format(format)
    .with_max_level(cli.verbosity())
    .with_target(false)
    .init();

  if let Err(err) = cli.execute().await {
    error!("{}", err);
    std::process::exit(1);
  }

  std::process::exit(0);
}
