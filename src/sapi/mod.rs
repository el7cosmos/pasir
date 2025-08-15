pub(crate) mod context;
mod ext;
mod util;

use crate::sapi::context::Context;
use crate::sapi::util::handle_abort_connection;
use bytes::{Bytes, BytesMut};
use ext_php_rs::builders::{ModuleBuilder, SapiBuilder};
use ext_php_rs::ffi::{
  ZEND_RESULT_CODE_FAILURE, ZEND_RESULT_CODE_SUCCESS, php_module_shutdown, php_module_startup,
  sapi_shutdown, sapi_startup, zend_error,
};
use ext_php_rs::types::Zval;
use ext_php_rs::zend::{SapiGlobals, SapiHeader, SapiModule};
use ext_php_rs::{php_function, php_module, wrap_function};
use headers::{HeaderMapExt, Host};
use hyper::Uri;
use hyper::header::{HeaderName, HeaderValue};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ops::Sub;
use std::str::FromStr;
use std::time::SystemTime;
use tracing::{debug, error, info, warn};
use util::register_variable;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sapi(pub(crate) *mut SapiModule);

unsafe impl Send for Sapi {}
unsafe impl Sync for Sapi {}

impl Sapi {
  pub(crate) fn new() -> Self {
    let builder = SapiBuilder::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_DESCRIPTION"))
      .startup_function(startup)
      .shutdown_function(shutdown)
      .ub_write_function(ub_write)
      .flush_function(flush)
      .send_header_function(send_header)
      .read_post_function(read_post)
      .read_cookies_function(read_cookies)
      .register_server_variables_function(register_server_variables)
      .log_message_function(log_message)
      .get_request_time_function(get_request_time);
    let mut sapi_module = builder.build().unwrap();
    sapi_module.sapi_error = Some(zend_error);
    Self(sapi_module.into_raw())
  }

  pub(crate) fn startup(self) -> Result<(), ()> {
    unsafe {
      sapi_startup(self.0);
      match (*self.0).startup {
        None => Ok(()),
        Some(startup) => match startup(self.0) {
          ZEND_RESULT_CODE_SUCCESS => Ok(()),
          ZEND_RESULT_CODE_FAILURE => Err(()),
          _ => Err(()),
        },
      }
    }
  }

  pub(crate) fn shutdown(self) {
    unsafe {
      if let Some(shutdown) = (*self.0).shutdown {
        shutdown(self.0);
      }
      sapi_shutdown()
    }
  }
}

extern "C" fn startup(sapi: *mut SapiModule) -> c_int {
  unsafe { php_module_startup(sapi, get_module()) }
}

extern "C" fn shutdown(_sapi: *mut SapiModule) -> c_int {
  unsafe { php_module_shutdown() };
  ZEND_RESULT_CODE_SUCCESS as c_int
}

extern "C" fn ub_write(str: *const c_char, str_length: usize) -> usize {
  let context = Context::from_server_context(SapiGlobals::get().server_context);
  if context.is_request_finished() {
    return 0;
  }

  let char = unsafe { std::slice::from_raw_parts(str.cast(), str_length) };
  match context.ub_write(Bytes::from(BytesMut::from(char))) {
    true => str_length,
    false => {
      handle_abort_connection();
      0
    }
  }
}

extern "C" fn flush(server_context: *mut c_void) {
  if !server_context.is_null() {
    Context::from_server_context(server_context).flush();
  }
}

extern "C" fn send_header(header: *mut SapiHeader, server_context: *mut c_void) {
  if header.is_null() {
    return;
  }

  let sapi_header = unsafe { *header };
  if sapi_header.header.is_null() {
    return;
  }

  let context = Context::from_server_context(server_context);
  if let Some(value) = sapi_header.value() {
    context.append_response_header(
      HeaderName::from_str(sapi_header.name()).unwrap(),
      HeaderValue::from_str(value).unwrap(),
    );
  }
}

extern "C" fn read_post(buffer: *mut c_char, length: usize) -> usize {
  let sapi_globals = SapiGlobals::get();

  let content_length = sapi_globals.request_info().content_length();
  if content_length == 0 {
    return 0;
  }

  // If we've read everything, return 0
  if sapi_globals.read_post_bytes >= content_length {
    return 0;
  }

  // Calculate how much we can read
  let to_read = length.min(content_length.sub(sapi_globals.read_post_bytes) as usize);

  let context = Context::from_server_context(sapi_globals.server_context);
  let bytes = context.body_mut().split_to(to_read);
  unsafe { buffer.copy_from(bytes.as_ptr().cast::<c_char>(), bytes.len()) }
  bytes.len()
}

extern "C" fn read_cookies() -> *mut c_char {
  Context::from_server_context(SapiGlobals::get().server_context)
    .headers()
    .get("Cookie")
    .map(|cookie| CString::new(cookie.to_str().unwrap()).unwrap().into_raw())
    .unwrap_or(std::ptr::null_mut())
}

extern "C" fn register_server_variables(vars: *mut Zval) {
  register_variable(
    "SERVER_SOFTWARE",
    format!(
      "{}/{} ({})",
      env!("CARGO_PKG_NAME"),
      env!("CARGO_PKG_VERSION"),
      env!("CARGO_PKG_DESCRIPTION"),
    ),
    vars,
  );

  let sapi_globals = SapiGlobals::get();
  let request_info = sapi_globals.request_info();
  if let Some(uri) = request_info.request_uri() {
    register_variable("REQUEST_URI", uri, vars);
  }
  if let Some(method) = request_info.request_method() {
    register_variable("REQUEST_METHOD", method, vars);
  }
  if let Some(query_string) = request_info.query_string() {
    register_variable("QUERY_STRING", query_string, vars);
  }

  let context = Context::from_server_context(sapi_globals.server_context);
  let root = context.root().to_str().unwrap_or_default();
  let script_name = context.route().script_name();
  let path_info = context.route().path_info();
  let php_self = format!("{}{}", script_name, path_info.unwrap_or_default());

  register_variable("PHP_SELF", php_self, vars);
  register_variable("SERVER_PROTOCOL", format!("{:?}", context.version()), vars);
  register_variable("DOCUMENT_ROOT", root, vars);
  register_variable("REMOTE_ADDR", context.peer_addr().ip().to_string(), vars);
  register_variable("REMOTE_PORT", context.peer_addr().port().to_string(), vars);
  register_variable("SCRIPT_FILENAME", format!("{root}{script_name}"), vars);
  register_variable("SERVER_ADDR", context.local_addr().ip().to_string(), vars);
  register_variable("SERVER_PORT", context.local_addr().port().to_string(), vars);
  register_variable("SCRIPT_NAME", script_name, vars);
  if let Some(path_info) = path_info {
    register_variable("PATH_INFO", path_info, vars);
  }

  let headers = context.headers();

  if let Ok(uri) = match headers.typed_get::<Host>() {
    None => Uri::from_maybe_shared(""),
    Some(host) => Uri::from_str(host.hostname()),
  } {
    register_variable("SERVER_NAME", uri.host().unwrap(), vars);
  }

  for (name, value) in headers.iter() {
    let header_name = format!("HTTP_{}", name.as_str().to_uppercase().replace('-', "_"));
    register_variable(header_name, value.to_str().unwrap(), vars);
  }
}

extern "C" fn log_message(message: *const c_char, syslog_type_int: c_int) {
  unsafe {
    let error_message = CStr::from_ptr(message);
    match syslog_type_int {
      0..=3 => error!("{error_message:?}"),
      4 => warn!("{error_message:?}"),
      5 | 6 => info!("{error_message:?}"),
      7 => debug!("{error_message:?}"),
      _ => (),
    };
  }
}

extern "C" fn get_request_time(time: *mut f64) -> c_int {
  unsafe { time.write(SystemTime::UNIX_EPOCH.elapsed().unwrap().as_secs_f64()) }
  ZEND_RESULT_CODE_SUCCESS as c_int
}

#[php_function]
fn fastcgi_finish_request() -> bool {
  Context::from_server_context(SapiGlobals::get().server_context).finish_request()
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
  module.function(wrap_function!(fastcgi_finish_request))
}
