use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CargoError;
use assert_cmd::cargo::CommandCargoExt;
use predicates::str::contains;
use predicates::str::starts_with;
use std::process::Command;

#[test]
fn test_cli_info() -> Result<(), CargoError> {
  let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
  cmd.arg("-i");
  cmd.assert().success().stdout(starts_with("phpinfo()"));

  Ok(())
}

#[test]
fn test_cli_module() -> Result<(), CargoError> {
  let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
  cmd.arg("-m");
  cmd.assert().success().stdout(contains("[PHP Modules]")).stdout(contains("[Zend Modules]"));

  Ok(())
}
