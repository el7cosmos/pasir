use std::ffi::CStr;

use ext_php_rs::zend::SapiGlobals;
use pasir_sapi::context::ServerContext;
use pasir_sys::PHP_INFO_ALL;
use pasir_sys::PHP_INFO_CREDITS;

use crate::cli::Executable;
use crate::sapi::Sapi;
use crate::sapi::context::Context;

#[derive(Default)]
pub struct Info {}

impl Executable for Info {
  async fn execute(self) -> anyhow::Result<()> {
    let context = Context::default();
    SapiGlobals::get_mut().server_context = context.into_raw().cast();

    Self::request_startup()?;
    unsafe { pasir_sys::php_print_info((PHP_INFO_ALL & !PHP_INFO_CREDITS).cast_signed()) }
    Self::request_shutdown();

    Ok(())
  }
}

impl ext_php_rs::embed::Sapi for Info {
  type Context = Context;

  fn name() -> &'static str {
    Sapi::name()
  }

  fn pretty_name() -> &'static str {
    Sapi::pretty_name()
  }

  fn ub_write(_ctx: &mut Self::Context, buf: &[u8]) -> usize {
    let mut bytes = buf.to_vec();
    if bytes.last() != Some(&0) {
      bytes.push(0);
    }

    match CStr::from_bytes_with_nul(&bytes) {
      Ok(s) => {
        print!("{}", s.to_string_lossy());
        buf.len()
      }
      Err(_) => 0,
    }
  }

  fn log_message(_message: &str, _syslog_type: i32) {}
}

impl pasir_sapi::Sapi for Info {
  fn php_info_as_text() -> bool {
    true
  }
}
