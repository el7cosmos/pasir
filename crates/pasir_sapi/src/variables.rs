use std::ffi::CStr;

/// `$_SERVER` variables.
/// https://www.php.net/manual/en/reserved.variables.server.php
pub static PHP_SELF: &CStr = c"PHP_SELF";
pub static SERVER_ADDR: &CStr = c"SERVER_ADDR";
pub static SERVER_NAME: &CStr = c"SERVER_NAME";
pub static SERVER_SOFTWARE: &CStr = c"SERVER_SOFTWARE";
pub static SERVER_PROTOCOL: &CStr = c"SERVER_PROTOCOL";
pub static REQUEST_METHOD: &CStr = c"REQUEST_METHOD";
pub static QUERY_STRING: &CStr = c"QUERY_STRING";
pub static DOCUMENT_ROOT: &CStr = c"DOCUMENT_ROOT";
pub static HTTPS: &CStr = c"HTTPS";
pub static REMOTE_ADDR: &CStr = c"REMOTE_ADDR";
pub static REMOTE_HOST: &CStr = c"REMOTE_HOST";
pub static REMOTE_PORT: &CStr = c"REMOTE_PORT";
pub static REMOTE_USER: &CStr = c"REMOTE_USER";
pub static SCRIPT_FILENAME: &CStr = c"SCRIPT_FILENAME";
pub static SERVER_PORT: &CStr = c"SERVER_PORT";
pub static SERVER_SIGNATURE: &CStr = c"SERVER_SIGNATURE";
pub static PATH_TRANSLATED: &CStr = c"PATH_TRANSLATED";
pub static SCRIPT_NAME: &CStr = c"SCRIPT_NAME";
pub static REQUEST_URI: &CStr = c"REQUEST_URI";
pub static AUTH_TYPE: &CStr = c"AUTH_TYPE";
pub static PATH_INFO: &CStr = c"PATH_INFO";
