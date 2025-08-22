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

#[cfg(test)]
mod tests {
  use crate::config::{Config, long_version, validate_root};
  use clap_verbosity_flag::{Verbosity, VerbosityFilter};
  use proptest::prelude::*;
  use std::net::Ipv4Addr;
  use std::path::PathBuf;

  proptest! {
    #[test]
    fn test_config(root: PathBuf, address: Ipv4Addr, port: u16, verbose in 0..3u8, quiet in 0..=3u8) {
      let config = Config {
        root: root.clone(),
        address: address.to_string(),
        port,
        verbosity: Verbosity::new(verbose, quiet),
      };

      assert_eq!(config.root(), root);
      assert_eq!(config.address(), address.to_string());
      assert_eq!(config.port(), port);

      let expected = match (verbose as i16) - (quiet as i16) {
        -3 => VerbosityFilter::Off,
        -2 => VerbosityFilter::Error,
        -1 => VerbosityFilter::Warn,
        0 => VerbosityFilter::Info,
        1 => VerbosityFilter::Debug,
        2 => VerbosityFilter::Trace,
        _ => unreachable!(),
      };
      assert_eq!(config.verbosity().filter(), expected);
    }
  }

  #[test]
  fn test_long_version() {
    assert!(long_version().starts_with(format!("{}\nPHP", env!("CARGO_PKG_VERSION")).as_str()));
  }

  #[test]
  fn test_validate_root() {
    // Not found.
    assert!(validate_root("./tests/foo").is_err());
    // Not a directory.
    assert!(validate_root("./tests/fixtures/routes.toml").is_err());
    // Root exists and is a directory.
    assert!(validate_root("tests/fixtures").is_ok());
  }
}
