use clap::Parser;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Parser)]
pub(crate) struct Config {
  #[arg(short, long, default_value = ".", value_parser = validate_root)]
  root: PathBuf,
  #[arg(short, long, default_value = "0.0.0.0")]
  address: String,
  #[arg(short, long, required = true)]
  port: u16,
  #[arg(short, long)]
  workers: Option<usize>,
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

  pub(crate) fn workers(&self) -> Option<usize> {
    self.workers
  }
}

fn validate_root(arg: &str) -> Result<PathBuf, String> {
  match PathBuf::from(arg).canonicalize() {
    Ok(root) => {
      if !root.is_dir() {
        return Err("root path is not a directory".to_string());
      }
      Ok(root)
    }
    Err(err) => Err(err.to_string()),
  }
}
