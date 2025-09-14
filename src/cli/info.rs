use std::ffi::c_int;

use ext_php_rs::ffi::PHP_INFO_ALL;
use ext_php_rs::ffi::PHP_INFO_CREDITS;
use ext_php_rs::ffi::ZEND_RESULT_CODE_FAILURE;
use ext_php_rs::ffi::ZEND_RESULT_CODE_SUCCESS;
use ext_php_rs::zend::ExecutorGlobals;
use pasir::error::PhpError;

use crate::cli::Executable;

pub struct Info {}

impl Executable for Info {
  async fn execute(self) -> anyhow::Result<()> {
    if unsafe { ext_php_rs::ffi::php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
      return Err(anyhow::anyhow!(PhpError::RequestStartupFailed));
    }

    unsafe { ext_php_rs::ffi::php_print_info((PHP_INFO_ALL & !PHP_INFO_CREDITS) as c_int) }

    ExecutorGlobals::get_mut().exit_status = ZEND_RESULT_CODE_SUCCESS;
    unsafe { ext_php_rs::ffi::php_output_end_all() }

    Ok(())
  }
}
