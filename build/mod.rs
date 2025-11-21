#[cfg(feature = "static")]
mod build_static;

use pasir_build::php_info::PHPInfo;

#[cfg(feature = "static")]
use crate::build_static::build_static;

fn main() -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=PHP");
  let php = pasir_build::find_executable("php", "PHP")?;
  let info = PHPInfo::get(&php)?;

  pasir_build::api_version::check_php_version(&info)?;

  println!("cargo::rustc-check-cfg=cfg(php_zend_max_execution_timers)");
  if info.zend_max_execution_timers()? {
    println!("cargo:rustc-cfg=php_zend_max_execution_timers");
  }

  #[cfg(feature = "static")]
  build_static(find_executable("spc", "SPC")?)?;

  Ok(())
}
