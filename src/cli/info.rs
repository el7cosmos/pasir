use pasir_sys::PHP_INFO_ALL;
use pasir_sys::PHP_INFO_CREDITS;

use crate::cli::Executable;

pub struct Info {}

impl Executable for Info {
  async fn execute(self) -> anyhow::Result<()> {
    Self::request_startup()?;
    unsafe { pasir_sys::php_print_info((PHP_INFO_ALL & !PHP_INFO_CREDITS).cast_signed()) }
    Self::request_shutdown();

    Ok(())
  }
}
