use ext_php_rs::builders::SapiBuilder;
use ext_php_rs::embed::SapiModule;

use crate::Sapi;

pub trait SapiBuilderExt {
  fn build_sapi_module<T: Sapi>(self) -> ext_php_rs::error::Result<SapiModule>;
}

impl SapiBuilderExt for SapiBuilder {
  fn build_sapi_module<T: Sapi>(self) -> ext_php_rs::error::Result<SapiModule> {
    let mut sapi_module = self.build()?;

    if sapi_module.startup.is_none() {
      sapi_module.startup = Some(T::startup);
    }
    if sapi_module.shutdown.is_none() {
      sapi_module.shutdown = Some(T::shutdown);
    }
    if sapi_module.deactivate.is_none() {
      sapi_module.deactivate = Some(T::deactivate);
    }
    sapi_module.sapi_error = Some(pasir_sys::zend_error);
    if sapi_module.log_message.is_none() {
      sapi_module.log_message = Some(T::log_message);
    }
    if sapi_module.get_request_time.is_none() {
      sapi_module.get_request_time = Some(T::get_request_time);
    }

    Ok(sapi_module)
  }
}
