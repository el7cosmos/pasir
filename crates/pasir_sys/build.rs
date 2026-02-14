use std::path::PathBuf;
use std::process::Command;

use cargo_manifest::Manifest;
use pasir_build::find_executable;

fn main() -> anyhow::Result<()> {
  println!("cargo:rerun-if-changed=Cargo.toml");
  println!("cargo:rerun-if-changed=pasir.h");

  let php_config = find_executable("php-config", "PHP_CONFIG")?;
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

  // Only macOS needs an explicit sysroot
  if std::env::var("CARGO_CFG_TARGET_OS") == Ok(String::from("macos")) {
    let sdk_path = Command::new("xcrun").arg("--show-sdk-path").output()?;

    if !sdk_path.status.success() {
      return Err(anyhow::anyhow!("xcrun failed to locate macOS SDK"));
    }

    let sdk = String::from_utf8(sdk_path.stdout)?.trim().to_string();

    builder = builder.clang_arg(format!("--sysroot={}", sdk));
  }

  let bindings = builder.generate()?;
  let out_path = PathBuf::from(std::env::var("OUT_DIR")?);
  bindings.write_to_file(out_path.join("bindings.rs"))?;

  Ok(())
}
