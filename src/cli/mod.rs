mod info;
mod module;
pub mod serve;

use std::ffi::CStr;
use std::path::PathBuf;

use clap_verbosity_flag::InfoLevel;
use clap_verbosity_flag::Verbosity;
use ext_php_rs::ffi::PHP_VERSION;
use ext_php_rs::ffi::ZEND_RESULT_CODE_FAILURE;
use ext_php_rs::ffi::ZEND_RESULT_CODE_SUCCESS;
use ext_php_rs::zend::ExecutorGlobals;
use pasir::error::PhpError;
use tokio::runtime::Handle;

use crate::cli::info::Info;
use crate::cli::module::Module;
use crate::cli::serve::Serve;
use crate::sapi::Sapi;

pub trait Executable {
  async fn execute(self) -> anyhow::Result<()>;

  fn request_startup() -> anyhow::Result<()> {
    if unsafe { ext_php_rs::ffi::php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
      return Err(anyhow::anyhow!(PhpError::RequestStartupFailed));
    }

    Ok(())
  }

  fn request_shutdown() {
    ExecutorGlobals::get_mut().exit_status = ZEND_RESULT_CODE_SUCCESS;
    unsafe { ext_php_rs::ffi::php_output_end_all() };
    unsafe { ext_php_rs::ffi::php_request_shutdown(std::ptr::null_mut()) };
  }
}

#[derive(Clone, Debug, clap::Parser)]
#[command(version, long_version = long_version(), about, author)]
pub struct Cli {
  #[arg(
    default_value_os_t = std::env::current_dir().unwrap_or(PathBuf::from(".")),
    value_parser = parse_root,
  )]
  root: PathBuf,
  #[arg(short, long, env = "PASIR_ADDRESS", default_value_os_t = std::net::Ipv4Addr::LOCALHOST.to_string())]
  address: String,
  #[arg(short, long, env = "PASIR_PORT", required_unless_present_any = vec!["info", "modules"])]
  port: Option<u16>,
  #[arg(short, long, help = "Define INI entry foo with value 'bar'", value_name = "foo[=bar]", value_parser = parse_define)]
  define: Vec<String>,
  #[arg(short, long, help = "PHP information and configuration", conflicts_with = "modules")]
  info: bool,
  #[arg(short, long, help = "Show compiled in modules", conflicts_with = "info")]
  modules: bool,
  #[command(flatten)]
  verbosity: Verbosity<InfoLevel>,
}

impl Cli {
  pub(crate) fn verbosity(&self) -> Verbosity<InfoLevel> {
    self.verbosity
  }

  fn shutdown(sapi: Sapi) {
    sapi.shutdown();
    unsafe { ext_php_rs::embed::ext_php_rs_sapi_shutdown() }
  }
}

impl Executable for Cli {
  async fn execute(self) -> anyhow::Result<()> {
    let expected_threads = Handle::current().metrics().num_workers().cast_signed();
    if !unsafe { ext_php_rs::ffi::php_tsrm_startup_ex(expected_threads.try_into()?) } {
      anyhow::bail!("Failed to start PHP TSRM");
    }

    let ini_entries = match self.define.is_empty() {
      true => None,
      false => Some(self.define.join("\n")),
    };

    let sapi = Sapi::new(self.info, ini_entries);
    if sapi.startup().is_err() {
      anyhow::bail!("Failed to start PHP SAPI module");
    };

    let result = if self.info {
      Info {}.execute().await
    } else if self.modules {
      Module {}.execute().await
    } else {
      Serve::new(self.address, self.port.expect("PORT argument were not provided"), self.root)
        .execute()
        .await
    };

    Self::shutdown(sapi);

    result
  }
}

fn long_version() -> String {
  format!(
    "{}\nPHP {}",
    env!("CARGO_PKG_VERSION"),
    CStr::from_bytes_with_nul(PHP_VERSION).unwrap().to_string_lossy()
  )
}

fn parse_root(arg: &str) -> Result<PathBuf, std::io::Error> {
  PathBuf::from(arg).canonicalize().and_then(|root| {
    if !root.is_dir() {
      return Err(std::io::Error::from(std::io::ErrorKind::NotADirectory));
    }
    Ok(root)
  })
}

fn parse_define(arg: &str) -> anyhow::Result<String> {
  if arg.split_once('=').is_some() { Ok(arg.to_string()) } else { Ok(format!("{arg}=On")) }
}

#[cfg(test)]
mod tests {
  use std::net::Ipv4Addr;
  use std::path::PathBuf;

  use clap_verbosity_flag::Verbosity;
  use clap_verbosity_flag::VerbosityFilter;
  use proptest::prelude::*;

  use crate::cli::Cli;
  use crate::cli::long_version;
  use crate::cli::parse_define;
  use crate::cli::parse_root;

  proptest! {
    #[test]
    fn test_config(root: PathBuf, address: Ipv4Addr, port: u16, verbose in 0..3u8, quiet in 0..=3u8) {
      let cli = Cli {
        root: root.clone(),
        address: address.to_string(),
        port: Some(port),
        define: vec![],
        info: false,
        modules: false,
        verbosity: Verbosity::new(verbose, quiet),
      };

      let expected = match (verbose as i16) - (quiet as i16) {
        -3 => VerbosityFilter::Off,
        -2 => VerbosityFilter::Error,
        -1 => VerbosityFilter::Warn,
        0 => VerbosityFilter::Info,
        1 => VerbosityFilter::Debug,
        2 => VerbosityFilter::Trace,
        _ => unreachable!(),
      };
      assert_eq!(cli.verbosity().filter(), expected);
    }
  }

  #[test]
  fn test_long_version() {
    assert!(long_version().starts_with(format!("{}\nPHP", env!("CARGO_PKG_VERSION")).as_str()));
  }

  #[test]
  fn test_parse_root() {
    // Not found.
    assert!(parse_root("./tests/foo").is_err());
    // Not a directory.
    assert!(parse_root("./tests/fixtures/routes.toml").is_err());
    // Root exists and is a directory.
    assert!(parse_root("tests/fixtures").is_ok());
  }

  #[test]
  fn test_parse_define() {
    assert_eq!(parse_define("foo").unwrap(), "foo=On");
    assert_eq!(parse_define("foo=bar").unwrap(), "foo=bar");
  }
}
