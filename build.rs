use anyhow::{Context, bail};
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
fn find_spc_build_json(json: &str) -> anyhow::Result<Vec<String>> {
  let buildroot = path_from_env("BUILD_ROOT_PATH").unwrap_or(PathBuf::from("buildroot"));
  if !buildroot.try_exists()? {
    bail!("spc buildroot not found at {:?}", buildroot);
  }
  let file = File::open(buildroot.join(json))?;
  Ok(serde_json::from_reader(BufReader::new(file))?)
}

#[cfg(feature = "static")]
fn build_spc() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=SPC");
  println!("cargo:rerun-if-env-changed=BUILD_ROOT_PATH");
  let spc = find_spc()?;
  let extensions = find_spc_build_json("build-extensions.json")?;
  let libraries = find_spc_build_json("build-libraries.json")?;

  let output = Command::new(spc)
    .arg("spc-config")
    .arg(extensions.join(","))
    .arg("--with-libs")
    .arg(libraries.join(","))
    .arg("--libs")
    .arg("--absolute-libs")
    .output()
    .expect("failed to run spc-config");

  let flags = String::from_utf8(output.stdout).expect("invalid UTF-8 from spc-config");

  let mut tokens = flags.split_whitespace().peekable();
  while let Some(token) = tokens.next() {
    if let Some(path) = token.strip_prefix("-L") {
      println!("cargo:rustc-link-search={}", path);
    } else if let Some(lib) = token.strip_prefix("-l") {
      println!("cargo:rustc-link-lib={lib}");
    } else if token.ends_with(".a") {
      println!("cargo:rustc-link-arg={token}");
    } else if token == "-framework"
      && let Some(name) = tokens.peek()
    {
      println!("cargo:rustc-link-lib=framework={name}");
      tokens.next();
    }
  }

  link_flags();

  Ok(())
}

#[cfg(all(target_os = "macos", feature = "static"))]
fn link_flags() {
  // Extra step only for Intel macOS (x86_64)
  #[cfg(target_arch = "x86_64")]
  {
    // Ask clang where its resource dir is (contains lib/darwin)
    if let Ok(output) = Command::new("clang").arg("--print-resource-dir").output() {
      if output.status.success() {
        if let Ok(dir) = String::from_utf8(output.stdout) {
          let dir = dir.trim();
          println!("cargo:rustc-link-search={}/lib/darwin", dir);
          println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
      }
    }
  }
}

#[cfg(all(target_env = "musl", feature = "static"))]
fn link_flags() {
  println!("cargo:rustc-link-arg=-fuse-ld=lld");
  println!("cargo:rustc-link-search=/usr/lib/clang/20/lib/linux");
  println!("cargo:rustc-link-lib=clang_rt.builtins-{}", std::env::consts::ARCH);
}

fn main() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=PASIR_VERSION");
  if let Ok(version) = std::env::var("PASIR_VERSION") {
    println!("cargo:rustc-env=CARGO_PKG_VERSION={version}");
  }

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
