// Portions of this code are derived from ext-php-rs
// Copyright (c) 2021 David Cole <david.cole1340@gmail.com> and all contributors
// Original source: https://github.com/davidcole1340/ext-php-rs
// Licensed under MIT License (see THIRD_PARTY_LICENSES file)

use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::Context;
use anyhow::bail;

/// Output of `php -i`.
pub(crate) struct PHPInfo(String);

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

  /// Get the zend version.
  ///
  /// # Errors
  /// - `PHPInfo` does not contain php api version
  pub fn zend_version(&self) -> anyhow::Result<u32> {
    self
      .get_key("PHP API")
      .context("Failed to get Zend version")
      .and_then(|s| u32::from_str(s).context("Failed to convert Zend version to integer"))
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
