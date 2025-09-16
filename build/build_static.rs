use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::Command;

use anyhow::bail;

pub(crate) fn build_static(spc: PathBuf) -> anyhow::Result<()> {
  println!("cargo:rerun-if-env-changed=SPC");
  println!("cargo:rerun-if-env-changed=BUILD_ROOT_PATH");

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

fn find_spc_build_json(json: &str) -> anyhow::Result<Vec<String>> {
  let buildroot = std::env::var_os("BUILD_ROOT_PATH")
    .map(PathBuf::from)
    // Default to `buildroot` in the current working directory.
    .unwrap_or(PathBuf::from("buildroot"));
  if !buildroot.try_exists()? {
    bail!("spc buildroot not found at {buildroot:?}");
  }
  let file = File::open(buildroot.join(json))?;
  Ok(serde_json::from_reader(BufReader::new(file))?)
}

#[cfg(target_os = "macos")]
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

#[cfg(target_env = "musl")]
fn link_flags() {
  println!("cargo:rustc-link-arg=-fuse-ld=lld");
  println!("cargo:rustc-link-search=/usr/lib/clang/20/lib/linux");
  println!("cargo:rustc-link-lib=clang_rt.builtins-{}", std::env::consts::ARCH);
}
