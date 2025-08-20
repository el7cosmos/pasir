use std::ffi::CStr;

/// `$_SERVER` variables.
/// https://www.php.net/manual/en/reserved.variables.server.php
pub(crate) static PHP_SELF: &CStr = c"PHP_SELF";
pub(crate) static SERVER_ADDR: &CStr = c"SERVER_ADDR";
pub(crate) static SERVER_NAME: &CStr = c"SERVER_NAME";
pub(crate) static SERVER_SOFTWARE: &CStr = c"SERVER_SOFTWARE";
pub(crate) static SERVER_PROTOCOL: &CStr = c"SERVER_PROTOCOL";
pub(crate) static REQUEST_METHOD: &CStr = c"REQUEST_METHOD";
pub(crate) static QUERY_STRING: &CStr = c"QUERY_STRING";
pub(crate) static DOCUMENT_ROOT: &CStr = c"DOCUMENT_ROOT";
// pub(crate) static HTTPS: &CStr = c"HTTPS";
pub(crate) static REMOTE_ADDR: &CStr = c"REMOTE_ADDR";
// pub(crate) static REMOTE_HOST: &CStr = c"REMOTE_HOST";
pub(crate) static REMOTE_PORT: &CStr = c"REMOTE_PORT";
// pub(crate) static REMOTE_USER: &CStr = c"REMOTE_USER";
pub(crate) static SCRIPT_FILENAME: &CStr = c"SCRIPT_FILENAME";
pub(crate) static SERVER_PORT: &CStr = c"SERVER_PORT";
// pub(crate) static SERVER_SIGNATURE: &CStr = c"SERVER_SIGNATURE";
// pub(crate) static PATH_TRANSLATED: &CStr = c"PATH_TRANSLATED";
pub(crate) static SCRIPT_NAME: &CStr = c"SCRIPT_NAME";
pub(crate) static REQUEST_URI: &CStr = c"REQUEST_URI";
// pub(crate) static AUTH_TYPE: &CStr = c"AUTH_TYPE";
pub(crate) static PATH_INFO: &CStr = c"PATH_INFO";
