// Portions of this code are derived from ext-php-rs
// Copyright (c) 2021 David Cole <david.cole1340@gmail.com> and all contributors
// Original source: https://github.com/davidcole1340/ext-php-rs
// Licensed under MIT License (see THIRD_PARTY_LICENSES file)

use std::cmp::PartialEq;
use std::cmp::PartialOrd;

use anyhow::Error;
use anyhow::anyhow;

use crate::php_info::PHPInfo;

#[derive(PartialEq, PartialOrd)]
enum ApiVersion {
  Php82 = 20220829,
  Php83 = 20230831,
  Php84 = 20240924,
}

impl ApiVersion {
  /// Returns the minimum supported API version.
  pub const fn min() -> Self {
    ApiVersion::Php82
  }

  /// Returns the maximum supported API version.
  pub const fn max() -> Self {
    ApiVersion::Php84
  }

  pub fn versions() -> Vec<Self> {
    vec![ApiVersion::Php82, ApiVersion::Php83, ApiVersion::Php84]
  }

  /// Returns the API versions that are supported by this version.
  pub fn supported_apis(self) -> Vec<ApiVersion> {
    ApiVersion::versions().into_iter().filter(|v| v <= &self).collect()
  }

  pub fn cfg_name(self) -> &'static str {
    match self {
      ApiVersion::Php82 => "php82",
      ApiVersion::Php83 => "php83",
      ApiVersion::Php84 => "php84",
    }
  }
}

impl TryFrom<u32> for ApiVersion {
  type Error = Error;

  fn try_from(version: u32) -> anyhow::Result<Self, Self::Error> {
    match version {
      x if ((ApiVersion::Php82 as u32)..(ApiVersion::Php83 as u32)).contains(&x) => Ok(ApiVersion::Php82),
      x if ((ApiVersion::Php83 as u32)..(ApiVersion::Php84 as u32)).contains(&x) => Ok(ApiVersion::Php83),
      x if (ApiVersion::Php84 as u32) == x => Ok(ApiVersion::Php84),
      version => Err(anyhow!(
        "The current version of PHP is not supported. Current PHP API version: {}, requires a version between {} and {}",
        version,
        ApiVersion::min() as u32,
        ApiVersion::max() as u32
      )),
    }
  }
}

/// Checks the PHP Zend API version and set any configuration flags required.
pub fn check_php_version(info: &PHPInfo) -> anyhow::Result<()> {
  let version = info.zend_version()?;
  let version: ApiVersion = version.try_into()?;

  println!("cargo::rustc-check-cfg=cfg(php82, php83, php84)");
  for supported_version in version.supported_apis() {
    println!("cargo:rustc-cfg={}", supported_version.cfg_name());
  }

  Ok(())
}
