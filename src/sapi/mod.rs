pub(crate) mod context;
mod ext;

use std::str::FromStr;

use bytes::Bytes;
use ext_php_rs::embed::SapiHeader;
use ext_php_rs::embed::SapiModule;
use ext_php_rs::embed::ServerContext as _;
use ext_php_rs::embed::ServerVarRegistrar;
use ext_php_rs::prelude::*;
use ext_php_rs::zend::FunctionEntry;
use ext_php_rs::zend::SapiGlobals;
use hyper::header::HeaderName;
use hyper::header::HeaderValue;
use pasir_sapi::context::ServerContext;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

use crate::sapi::context::Context;

pub struct Sapi;

impl ext_php_rs::embed::Sapi for Sapi {
  type Context = Context;

  fn name() -> &'static str {
    env!("CARGO_PKG_NAME")
  }

  fn pretty_name() -> &'static str {
    env!("CARGO_PKG_DESCRIPTION")
  }

  fn ub_write(ctx: &mut Self::Context, buf: &[u8]) -> usize {
    if ctx.is_request_finished() {
      return 0;
    }

    match ctx.ub_write(Bytes::copy_from_slice(buf)) {
      true => buf.len(),
      false => {
        pasir_sapi::util::handle_abort_connection();
        0
      }
    }
  }

  fn log_message(message: &str, syslog_type: i32) {
    match syslog_type {
      0..=3 => error!("{message}"),
      4 => warn!("{message}"),
      5 | 6 => info!("{message}"),
      7 => debug!("{message}"),
      _ => (),
    };
  }

  fn flush(ctx: &mut Self::Context) {
    ctx.flush();
  }

  fn send_header(ctx: &mut Self::Context, header: &SapiHeader) {
    if header.is_empty() {
      return;
    }

    if let Some((name, value)) = header.as_name_value()
      && let Ok(name) = HeaderName::from_str(name)
      && let Ok(value) = HeaderValue::from_str(value)
    {
      ctx.append_response_header(name, value);
    }
  }

  fn register_server_variables(ctx: &mut Self::Context, registrar: &mut ServerVarRegistrar) {
    ctx.register_server_variables(registrar);
  }
}

impl pasir_sapi::Sapi for Sapi {
  fn build_module() -> ext_php_rs::error::Result<SapiModule>
  where
    Self: Sized,
    Self::Context: ServerContext,
  {
    let mut sapi_module = <Self as ext_php_rs::embed::Sapi>::build_module()?;

    sapi_module.startup = Some(Self::startup);
    sapi_module.shutdown = Some(Self::shutdown);
    sapi_module.deactivate = Some(Self::deactivate);
    sapi_module.sapi_error = Some(pasir_sys::zend_error);
    sapi_module.phpinfo_as_text = Self::php_info_as_text().into();

    let function_entry = wrap_function!(pasir_finish_request).build()?;
    let mut function_alias = function_entry;
    function_alias.fname = c"fastcgi_finish_request".as_ptr();

    let functions = vec![function_entry, function_alias, FunctionEntry::end()];
    sapi_module.additional_functions = Box::into_raw(functions.into_boxed_slice()).cast();

    Ok(sapi_module)
  }
}

#[php_function]
fn pasir_finish_request() -> bool {
  Context::from_server_context(SapiGlobals::get().server_context).finish_request()
}
