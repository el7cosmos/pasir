use thiserror::Error;

#[derive(Debug, Error)]
pub enum PhpError {
  #[error("Request startup failed")]
  RequestStartupFailed,
  #[error("Server context corrupted during execution")]
  ServerContextCorrupted,
}
