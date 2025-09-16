use ext_php_rs::ffi::PHP_INFO_ALL;
use ext_php_rs::ffi::PHP_INFO_CREDITS;

use crate::cli::Executable;

pub struct Info {}

impl Executable for Info {
  async fn execute(self) -> anyhow::Result<()> {
    Self::request_startup()?;
    unsafe { ext_php_rs::ffi::php_print_info((PHP_INFO_ALL & !PHP_INFO_CREDITS).cast_signed()) }
    Self::request_shutdown();

    Ok(())
  }
}
