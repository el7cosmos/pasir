use anyhow::{Context, bail};
use std::env::var;
#[cfg(feature = "static")]
use std::fs::File;
#[cfg(feature = "static")]
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Output of `php -i`.
pub struct PHPInfo(String);

impl PHPInfo {
  /// Get the PHP info.
  ///
  /// # Errors
  /// - `phpinfo()` failed to execute successfully
  pub fn get(php: &Path) -> anyhow::Result<Self> {
    let cmd = Command::new(php)
      .arg("-r")
      .arg("phpinfo(INFO_GENERAL);")
      .output()
      .context("Failed to call `phpinfo()`")?;
    let stdout = String::from_utf8_lossy(&cmd.stdout);
    if !cmd.status.success() {
      bail!("Failed to call `phpinfo()` status code {}", cmd.status);
    }
    Ok(Self(stdout.to_string()))
  }

  /// Checks if zend max execution timers is enabled.
  ///
  /// # Errors
  /// - `PHPInfo` does not contain zend max execution timers information
  pub fn zend_max_execution_timers(&self) -> anyhow::Result<bool> {
    Ok(
      self
        .get_key("Zend Max Execution Timers")
        .context("Could not find zend max execution timers of PHP")?
        == "enabled",
    )
  }

  fn get_key(&self, key: &str) -> Option<&str> {
    let split = format!("{key} => ");
    for line in self.0.lines() {
      let components: Vec<_> = line.split(&split).collect();
      if components.len() > 1 {
        return Some(components[1]);
      }
    }
    None
  }
}

/// Finds the location of an executable `name`.
#[must_use]
pub fn find_executable(name: &str) -> Option<PathBuf> {
  const WHICH: &str = if cfg!(windows) { "where" } else { "which" };
  let cmd = Command::new(WHICH).arg(name).output().ok()?;
  if cmd.status.success() {
    let stdout = String::from_utf8_lossy(&cmd.stdout);
    stdout.trim().lines().next().map(|l| l.trim().into())
  } else {
    None
  }
}

/// Returns an environment variable's value as a `PathBuf`
pub fn path_from_env(key: &str) -> Option<PathBuf> {
  std::env::var_os(key).map(PathBuf::from)
}

/// Finds the location of the PHP executable.
fn find_php() -> anyhow::Result<PathBuf> {
  // If path is given via env, it takes priority.
  if let Some(path) = path_from_env("PHP") {
    if !path.try_exists()? {
      // If path was explicitly given and it can't be found, this is a hard error
      bail!("php executable not found at {:?}", path);
    }
    return Ok(path);
  }
  find_executable("php").with_context(|| {
    "Could not find PHP executable. \
    Please ensure `php` is in your PATH or the `PHP` environment variable is set."
  })
}

#[cfg(feature = "static")]
fn find_spc() -> anyhow::Result<PathBuf> {
  if let Some(path) = path_from_env("SPC") {
    if !path.try_exists()? {
      bail!("spc executable not found at {:?}", path);
    }
    return Ok(path);
  }
  find_executable("spc").with_context(|| {
    "Could not find SPC executable. \
    Please ensure `spc` is in your PATH or the `SPC` environment variable is set."
  })
}

#[cfg(feature = "static")]
fn build_spc() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=SPC");
  let spc = find_spc()?;
  let file = File::open("buildroot/build-extensions.json")?;
  let reader = BufReader::new(file);
  let extensions: Vec<String> = serde_json::from_reader(reader)?;

  let output = Command::new(spc)
    .arg("spc-config")
    .arg(extensions.join(","))
    .arg("--libs")
    .output()
    .expect("failed to run spc-config");

  let flags = String::from_utf8(output.stdout).expect("invalid UTF-8 from spc-config");

  for token in flags.split_whitespace() {
    if let Some(path) = token.strip_prefix("-L") {
      println!("cargo:rustc-link-search={}", path);
    } else if let Some(lib) = token.strip_prefix("-l") {
      println!("cargo:rustc-link-lib={}", lib);
    } else if token == "-framework" {
      // handled in next iteration, see below
    }
  }

  // Special handling for `-framework X` pairs
  let mut tokens = flags.split_whitespace().peekable();
  while let Some(token) = tokens.next() {
    if token == "-framework" {
      if let Some(name) = tokens.peek() {
        println!("cargo:rustc-link-lib=framework={name}");
        tokens.next();
      }
    }
  }

  Ok(())
}

fn main() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=PASIR_VERSION");
  let version = var("PASIR_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
  println!("cargo:rustc-env=PASIR_VERSION={version}");

  println!("cargo::rustc-check-cfg=cfg(php_zend_max_execution_timers)");
  let php = find_php()?;
  let info = PHPInfo::get(&php)?;
  if info.zend_max_execution_timers()? {
    println!("cargo:rustc-cfg=php_zend_max_execution_timers");
  }

  #[cfg(feature = "static")]
  build_spc()?;

  Ok(())
}
