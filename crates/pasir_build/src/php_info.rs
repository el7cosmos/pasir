use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::Context;
use anyhow::bail;

/// Output of `php -i`.
pub struct PHPInfo(String);

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

  /// Checks if thread safety is enabled.
  ///
  /// # Errors
  /// - `PHPInfo` does not contain thread safety information
  pub fn thread_safety(&self) -> anyhow::Result<bool> {
    Ok(self.get_key("Thread Safety").context("Could not find thread safety of PHP")? == "enabled")
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
    let zend_max_execution_timers_value = self
      .get_key("Zend Max Execution Timers")
      .context("Could not find zend max execution timers of PHP");
    Ok(zend_max_execution_timers_value? == "enabled")
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
