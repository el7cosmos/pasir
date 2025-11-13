mod api_version;
#[cfg(feature = "static")]
mod build_static;
mod php_info;

use crate::api_version::check_php_version;
#[cfg(feature = "static")]
use crate::build_static::build_static;
use crate::php_info::PHPInfo;

fn main() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=PHP");
  let php = pasir_build::find_executable("php", "PHP")?;
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
