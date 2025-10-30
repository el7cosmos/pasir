mod api_version;
#[cfg(feature = "static")]
mod build_static;
mod php_info;

use std::path::PathBuf;
use std::process::Command;

use cargo_manifest::Manifest;

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

  println!("cargo:rerun-if-changed=Cargo.toml");
  println!("cargo:rerun-if-changed=build/mod.rs");
  println!("cargo:rerun-if-changed=pasir.h");

  let php_config = pasir_build::find_executable("php-config", "PHP_CONFIG")?;
  let cmd = Command::new(php_config).arg("--includes").output()?;
  let stdout = String::from_utf8_lossy(&cmd.stdout);

  let manifest = Manifest::from_path(std::env::var("CARGO_MANIFEST_PATH")?)?;
  let metadata = manifest.package.unwrap().metadata.unwrap();
  let allowlist_item = metadata.get("allowlist_item").unwrap().as_array().unwrap();
  let blocklist_item = metadata.get("blocklist_Item").unwrap().as_array().unwrap();

  let mut builder = bindgen::Builder::default()
    .header("pasir.h")
    .clang_args(stdout.split(' '))
    .derive_default(true)
    .generate_cstr(true);
  for item in allowlist_item {
    builder = builder.allowlist_item(item.as_str().unwrap());
  }
  for item in blocklist_item {
    builder = builder.blocklist_item(item.as_str().unwrap());
  }

  let bindings = builder.generate()?;
  let out_path = PathBuf::from(std::env::var("OUT_DIR")?);
  bindings.write_to_file(out_path.join("bindings.rs"))?;

  Ok(())
}
