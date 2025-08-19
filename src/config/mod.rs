pub(crate) mod route;

use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use ext_php_rs::ffi::PHP_VERSION;
use std::ffi::CStr;
use std::path::PathBuf;

#[derive(Clone, Debug, Parser)]
#[command(version, long_version = long_version(), about, author)]
pub(crate) struct Config {
  #[arg(
    default_value_os_t = std::env::current_dir().unwrap_or(PathBuf::from(".")),
    value_parser = validate_root,
  )]
  root: PathBuf,
  #[arg(short, long, env = "PASIR_ADDRESS", default_value_os_t = std::net::Ipv4Addr::LOCALHOST.to_string())]
  address: String,
  #[arg(short, long, env = "PASIR_PORT", required = true)]
  port: u16,
  #[command(flatten)]
  verbosity: Verbosity<InfoLevel>,
}

impl Config {
  pub(crate) fn root(&self) -> PathBuf {
    self.root.clone()
  }

  pub(crate) fn address(&self) -> &str {
    &self.address
  }

  pub(crate) fn port(&self) -> u16 {
    self.port
  }

  pub(crate) fn verbosity(&self) -> Verbosity<InfoLevel> {
    self.verbosity
  }
}

fn long_version() -> String {
  format!(
    "{}\nPHP {}",
    env!("CARGO_PKG_VERSION"),
    CStr::from_bytes_with_nul(PHP_VERSION).unwrap().to_string_lossy()
  )
}

fn validate_root(arg: &str) -> Result<PathBuf, std::io::Error> {
  PathBuf::from(arg).canonicalize().and_then(|root| {
    if !root.is_dir() {
      return Err(std::io::Error::from(std::io::ErrorKind::NotADirectory));
    }
    Ok(root)
  })
}
