use std::process::Command;

use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CargoError;
use assert_cmd::cargo::CommandCargoExt;
use predicates::str::contains;
use predicates::str::starts_with;

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
  cmd
    .assert()
    .success()
    .stdout(contains("[PHP Modules]"))
    .stdout(contains("[Zend Modules]"));

  Ok(())
}

#[test]
fn test_cli_define() -> Result<(), CargoError> {
  let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
  cmd.arg("-i").arg("-dassert.active=Off").arg("-d").arg("assert.bail");
  cmd
    .assert()
    .success()
    .stdout(contains("assert.active => Off => Off"))
    .stdout(contains("assert.bail => On => On"));

  let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
  cmd.arg("-i").arg("--define").arg("assert.active=Off").arg("-dassert.bail");
  cmd
    .assert()
    .success()
    .stdout(contains("assert.active => Off => Off"))
    .stdout(contains("assert.bail => On => On"));

  Ok(())
}
