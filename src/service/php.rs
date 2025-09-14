use std::convert::Infallible;
use std::ffi::CString;
use std::ffi::NulError;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use bytes::Bytes;
use ext_php_rs::embed::Embed;
use ext_php_rs::embed::ext_php_rs_sapi_per_thread_init;
use ext_php_rs::ffi::ZEND_RESULT_CODE_FAILURE;
use ext_php_rs::ffi::php_handle_auth_data;
use ext_php_rs::ffi::php_request_shutdown;
use ext_php_rs::ffi::php_request_startup;
use ext_php_rs::zend::SapiGlobals;
use ext_php_rs::zend::try_catch_first;
use headers::ContentLength;
use headers::ContentType;
use headers::HeaderMapExt;
use http_body_util::BodyExt;
use http_body_util::Empty;
use http_body_util::Full;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::Version;
use hyper::body::Incoming;
use hyper::http::request::Parts;
use pasir::error::PhpError;
use tower::Service;
use tracing::error;
use tracing::instrument;
use tracing::trace;

use crate::cli::serve::Stream;
use crate::sapi::context::Context;
use crate::sapi::context::ContextSender;
use crate::sapi::context::ResponseType;
use crate::util::response_ext::ResponseExt;

#[derive(Clone, Default)]
pub(crate) struct PhpService {}

impl Service<Request<Incoming>> for PhpService {
  type Response = Response<UnsyncBoxBody<Bytes, Infallible>>;
  type Error = Infallible;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, req: Request<Incoming>) -> Self::Future {
    let root = req.extensions().get::<Arc<PathBuf>>().unwrap().clone();
    let stream = req.extensions().get::<Arc<Stream>>().unwrap().clone();
    let error_body = Empty::default().boxed_unsync();

    Box::pin(async move {
      let (head, body) = req.into_parts();
      let route = resolve_php_index(root.as_path(), head.uri.path());
      let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Response::internal_server_error(error_body),
      };

      unsafe { ext_php_rs_sapi_per_thread_init() }
      let path_translated = format!("{}{}", root.to_str().unwrap(), route.script_name());
      if init_sapi_globals(&head, path_translated.as_str()).is_err() {
        return Response::bad_request(error_body);
      }

      let (head_rx, body_rx, context_tx) = ContextSender::receiver();
      let request = Request::from_parts(head, bytes);
      let context = Context::new(root, route, stream, request, context_tx);
      if let Err(err) = execute_php(context) {
        return match err {
          PhpError::RequestStartupFailed => Response::service_unavailable(error_body),
          PhpError::ServerContextCorrupted => Response::internal_server_error(error_body),
        };
      }

      match head_rx.await {
        Ok(mut head) => {
          let response_type = head.extensions.get_or_insert_default::<ResponseType>();
          let body = match response_type {
            ResponseType::Full => {
              Full::new(body_rx.collect().await.unwrap().to_bytes()).boxed_unsync()
            }
            ResponseType::Chunked => body_rx.boxed_unsync(),
          };
          let response = Response::from_parts(head, body);
          Ok(response)
        }
        Err(_) => Response::internal_server_error(error_body),
      }
    })
  }
}

/// Represents a resolved PHP route
#[derive(Debug, Clone, PartialEq)]
pub struct PhpRoute {
  script_name: String,
  path_info: Option<String>,
}

impl PhpRoute {
  pub(crate) fn new(script_name: String, path_info: Option<String>) -> Self {
    Self { script_name, path_info }
  }

  pub(crate) fn script_name(&self) -> &str {
    &self.script_name
  }

  pub(crate) fn path_info(&self) -> Option<&str> {
    self.path_info.as_deref()
  }
}

#[instrument(skip(head, path_translated), err)]
fn init_sapi_globals(head: &Parts, path_translated: &str) -> Result<(), NulError> {
  let mut sapi_globals = SapiGlobals::get_mut();
  sapi_globals.sapi_headers.http_response_code = StatusCode::OK.as_u16() as c_int;
  sapi_globals.request_info.request_method = CString::new(head.method.as_str())?.into_raw();
  sapi_globals.request_info.query_string = head
    .uri
    .query()
    .and_then(|query| CString::new(query).ok())
    .map(|query| query.into_raw())
    .unwrap_or_else(std::ptr::null_mut);
  sapi_globals.request_info.path_translated = CString::new(path_translated)?.into_raw();
  sapi_globals.request_info.request_uri = CString::new(head.uri.to_string())?.into_raw();
  sapi_globals.request_info.content_length = head
    .headers
    .typed_get::<ContentLength>()
    .map_or(0, |content_length| content_length.0.cast_signed());
  sapi_globals.request_info.content_type =
    head.headers.typed_get::<ContentType>().map_or(Ok(std::ptr::null_mut()), |content_type| {
      CString::new(content_type.to_string()).map(|content_type| content_type.into_raw())
    })?;

  if let Some(auth) = head.headers.get("Authorization") {
    unsafe {
      php_handle_auth_data(CString::new(auth.as_bytes())?.as_ptr());
    }
  }

  let proto_num = match head.version {
    Version::HTTP_09 => 900,
    Version::HTTP_10 => 1000,
    Version::HTTP_11 => 1100,
    Version::HTTP_2 => 2000,
    Version::HTTP_3 => 3000,
    _ => unreachable!(),
  };
  sapi_globals.request_info.proto_num = proto_num;

  Ok(())
}

#[instrument(skip(context), err)]
fn execute_php(context: Context) -> Result<(), PhpError> {
  let script = context.root().join(context.route().script_name().trim_start_matches("/"));

  let context_raw = context.into_raw();
  SapiGlobals::get_mut().server_context = context_raw.cast::<c_void>();

  if unsafe { php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
    return Err(PhpError::RequestStartupFailed);
  }

  let catch = try_catch_first(|| {
    if let Err(e) = Embed::run_script(script.as_path())
      && e.is_bailout()
    {
      error!("run_script failed: {:?}", e);
    }
  });

  request_shutdown();

  if let Err(e) = catch {
    error!("catch failed: {:?}", e);
  }

  // Validate server_context before using
  let server_context = SapiGlobals::get().server_context;
  if server_context.is_null() || server_context != context_raw.cast::<c_void>() {
    return Err(PhpError::ServerContextCorrupted);
  }

  let mut context = unsafe { Context::from_raw(server_context) };
  if !context.is_request_finished() && !context.finish_request() {
    trace!("finish request failed");
  }

  Ok(())
}

fn request_shutdown() {
  unsafe { php_request_shutdown(std::ptr::null_mut()) }

  let mut request_info = SapiGlobals::get().request_info;

  free_ptr(&mut request_info.request_method.cast_mut());
  free_ptr(&mut request_info.query_string);
  free_ptr(&mut request_info.path_translated);
  free_ptr(&mut request_info.request_uri);
  free_ptr(&mut request_info.content_type.cast_mut());
  free_ptr(&mut request_info.cookie_data);
}

// Helper to free and null a pointer we allocated with into_raw()
fn free_ptr(ptr: &mut *mut std::os::raw::c_char) {
  if !ptr.is_null() {
    let _ = unsafe { CString::from_raw(*ptr) };
    *ptr = std::ptr::null_mut();
  }
}

/// Resolves a request URI to the appropriate PHP file and path info
/// Returns PhpRoute with script_name and path_info
fn resolve_php_index(document_root: &Path, request_uri: &str) -> PhpRoute {
  // Clean the request URI (remove query string, normalize slashes)
  let clean_uri = request_uri.split('?').next().unwrap_or(request_uri);
  let clean_uri = clean_uri.trim_start_matches('/').trim_end_matches('/');

  // Split the path into segments
  let segments: Vec<&str> = if clean_uri.is_empty() {
    vec![]
  } else {
    clean_uri.split('/').filter(|s| !s.is_empty()).collect()
  };

  // Check if the URI explicitly contains a .php file
  if let Some(php_pos) = segments.iter().position(|&s| s.ends_with(".php")) {
    // URI contains a PHP file explicitly
    let script_path_segments = &segments[0..php_pos + 1]; // Include the .php file
    let path_info_segments = &segments[php_pos + 1..];

    // Build the script path
    let script_path = format!("/{}", script_path_segments.join("/"));

    // Build path info
    let path_info = if path_info_segments.is_empty() {
      None
    } else {
      Some(format!("/{}", path_info_segments.join("/")))
    };

    // Verify the PHP file actually exists
    let mut file_path = document_root.to_path_buf();
    for segment in script_path_segments {
      file_path.push(segment);
    }

    if file_path.exists() && file_path.is_file() {
      return PhpRoute::new(script_path, path_info);
    }
  }

  // Try to find index.php by traversing from the most specific to the least specific path
  for i in (0..=segments.len()).rev() {
    let current_path_segments = &segments[0..i];
    let remaining_segments = &segments[i..];

    // Build the directory path to check
    let mut dir_path = document_root.to_path_buf();
    for segment in current_path_segments {
      dir_path.push(segment);
    }

    // Check if index.php exists in this directory
    let index_php_path = dir_path.join("index.php");
    if index_php_path.exists() && index_php_path.is_file() {
      // Found index.php, construct the paths
      let relative_index_path = if current_path_segments.is_empty() {
        "/index.php".to_string()
      } else {
        format!("/{}/index.php", current_path_segments.join("/"))
      };

      let path_info = if remaining_segments.is_empty() {
        None
      } else {
        Some(format!("/{}", remaining_segments.join("/")))
      };

      return PhpRoute::new(relative_index_path, path_info);
    }
  }

  // Fallback to root index.php if it exists
  let root_index = document_root.join("index.php");
  if root_index.exists() && root_index.is_file() {
    let path_info = if clean_uri.is_empty() { None } else { Some(format!("/{clean_uri}")) };
    PhpRoute::new("/index.php".to_string(), path_info)
  } else {
    // No index.php found anywhere
    PhpRoute::new("/index.php".to_string(), Some(format!("/{clean_uri}")))
  }
}

#[cfg(test)]
mod tests {
  use std::fs;

  use hyper::Method;
  use hyper::Uri;
  use hyper::header::CONTENT_LENGTH;
  use hyper::header::CONTENT_TYPE;

  use super::*;
  use crate::sapi::tests::TestSapi;

  #[test]
  fn test_init_sapi_globals() {
    let _guard = TestSapi::new();

    let uri = Uri::builder().path_and_query("/foo?bar=baz").build().unwrap();
    let (head, _) = Request::builder()
      .method(Method::POST)
      .version(Version::HTTP_3)
      .header(CONTENT_LENGTH, "Foo Bar".len())
      .header(CONTENT_TYPE, "text/plain")
      .uri(uri)
      .body(Bytes::default())
      .unwrap()
      .into_parts();
    let results = init_sapi_globals(&head, "./index.php");
    assert!(results.is_ok());

    let sapi_globals = SapiGlobals::get();
    assert_eq!(sapi_globals.sapi_headers().http_response_code, StatusCode::OK.as_u16() as c_int);

    let request_info = sapi_globals.request_info();
    assert_eq!(request_info.request_method(), Some(Method::POST.as_str()));
    assert_eq!(request_info.query_string(), Some("bar=baz"));
    assert_eq!(request_info.path_translated(), Some("./index.php"));
    assert_eq!(request_info.request_uri(), Some("/foo?bar=baz"));
    assert_eq!(request_info.content_length(), "Foo Bar".len() as i64);
    assert_eq!(request_info.content_type(), Some("text/plain"));
    assert_eq!(request_info.auth_user(), None);
    assert_eq!(request_info.proto_num(), 3000);
  }

  #[test]
  fn test_resolve_php_index() {
    // Create a temporary directory structure for testing
    let temp_dir = std::env::temp_dir().join("php_test");
    let _ = fs::remove_dir_all(&temp_dir); // Clean up if exists
    fs::create_dir_all(&temp_dir).unwrap();
    fs::create_dir_all(temp_dir.join("some_path")).unwrap();

    // Create index.php files and a status.php file
    fs::write(temp_dir.join("index.php"), "<?php echo 'root'; ?>").unwrap();
    fs::write(temp_dir.join("some_path/index.php"), "<?php echo 'some_path'; ?>").unwrap();
    fs::write(temp_dir.join("some_path/status.php"), "<?php echo 'status'; ?>").unwrap();

    // Test cases
    let test_cases = vec![
      ("/", PhpRoute::new("/index.php".to_string(), None)),
      (
        "/some_path/admin",
        PhpRoute::new("/some_path/index.php".to_string(), Some("/admin".to_string())),
      ),
      (
        "/some_other_path/admin",
        PhpRoute::new("/index.php".to_string(), Some("/some_other_path/admin".to_string())),
      ),
      ("/some_path/", PhpRoute::new("/some_path/index.php".to_string(), None)),
      ("/some_path", PhpRoute::new("/some_path/index.php".to_string(), None)),
      (
        "/some_path/index.php/admin",
        PhpRoute::new("/some_path/index.php".to_string(), Some("/admin".to_string())),
      ),
      (
        "/some_path/status.php/admin",
        PhpRoute::new("/some_path/status.php".to_string(), Some("/admin".to_string())),
      ),
    ];

    for (uri, expected_route) in test_cases {
      let route = resolve_php_index(&temp_dir, uri);
      assert_eq!(route, expected_route, "Failed for URI: {}", uri);
    }

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
  }
}
