use std::ffi::NulError;

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ExecutePhpError {
  #[error(transparent)]
  InitSapiGlobalsError(#[from] NulError),
  #[error("Request startup failed")]
  RequestStartupFailed,
  #[error("A bailout occurred during the execution")]
  Bailout,
}
