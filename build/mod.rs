mod api_version;
#[cfg(feature = "static")]
mod build_static;
mod php_info;

use std::path::PathBuf;

use anyhow::Context;
use anyhow::bail;

use crate::api_version::check_php_version;
#[cfg(feature = "static")]
use crate::build_static::build_static;
use crate::php_info::PHPInfo;

/// Finds the location of an executable `name`.
pub fn find_executable(name: &str, env_name: &str) -> anyhow::Result<PathBuf> {
  if let Some(path) = std::env::var_os(env_name).map(PathBuf::from) {
    if !path.try_exists()? {
      // If path was explicitly given and it can't be found, this is a hard error
      bail!("{name} executable not found at {path:?}");
    }
    return Ok(path);
  }
  which::which(name).with_context(|| {
    format!(
      "Could not find {env_name} executable. \
      Please ensure `{name}` is in your PATH or the `{env_name}` environment variable is set."
    )
  })
}

fn main() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=PHP");
  let php = find_executable("php", "PHP")?;
  let info = PHPInfo::get(&php)?;

  check_php_version(&info)?;

  println!("cargo::rustc-check-cfg=cfg(php_zend_max_execution_timers)");
  if info.zend_max_execution_timers()? {
    println!("cargo:rustc-cfg=php_zend_max_execution_timers");
  }

  #[cfg(feature = "static")]
  build_static(find_executable("spc", "SPC")?)?;

  Ok(())
}
