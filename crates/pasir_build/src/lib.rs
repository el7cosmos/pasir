pub mod api_version;
pub mod php_info;

use std::path::PathBuf;

use anyhow::Context;

/// Finds the location of an executable `name`.
pub fn find_executable(name: &str, env_name: &str) -> anyhow::Result<PathBuf> {
  if let Some(path) = std::env::var_os(env_name).map(PathBuf::from) {
    if !path.try_exists()? {
      // If path was explicitly given and it can't be found, this is a hard error
      anyhow::bail!("{name} executable not found at {path:?}");
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

#[cfg(test)]
mod test {
  use std::path::PathBuf;

  use crate::find_executable;

  #[test]
  fn test_find_executable_no_env() {
    assert!(find_executable("foo", "FOO").is_err());
  }

  #[test]
  fn test_find_executable_not_exist() {
    unsafe { std::env::set_var("FOO", PathBuf::from("/foo")) };
    assert!(find_executable("foo", "FOO").is_err());
  }

  #[test]
  fn test_find_executable() -> anyhow::Result<()> {
    let path = PathBuf::from("tests/fixtures/foo");
    unsafe { std::env::set_var("FOO", path.as_path()) };
    assert_eq!(find_executable("foo", "FOO")?.as_path(), path.as_path());
    Ok(())
  }
}
