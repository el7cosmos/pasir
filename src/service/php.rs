use crate::response::InternalServerError;
use crate::sapi::context::Context;
use anyhow::Error;
use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use tokio::sync::mpsc::Sender;
use tower::Service;

#[derive(Clone)]
pub(crate) struct PhpService {
  local_addr: SocketAddr,
  peer_addr: SocketAddr,
  sender: Sender<Context>,
}

impl PhpService {
  pub(crate) fn new(
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    sender: Sender<Context>,
  ) -> Self {
    Self { local_addr, peer_addr, sender }
  }
}

impl Service<Request<Incoming>> for PhpService {
  type Response = Response<UnsyncBoxBody<Bytes, Error>>;
  type Error = Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, request: Request<Incoming>) -> Self::Future {
    let root = request.extensions().get::<Arc<PathBuf>>().unwrap().clone();
    let local_addr = self.local_addr;
    let peer_addr = self.peer_addr;
    let sender = self.sender.clone();

    Box::pin(async move {
      let (head, body) = request.into_parts();
      let bytes = body.collect().await?.to_bytes();
      let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
      let route = resolve_php_index(root.as_path(), head.uri.path());
      let error_response = Response::internal_server_error(UnsyncBoxBody::default());

      let context = Context::new(
        root,
        route,
        local_addr,
        peer_addr,
        Request::from_parts(head, bytes),
        Some(resp_tx),
      );

      if sender.send(context).await.is_err() {
        return Ok(error_response.unwrap());
      }

      match resp_rx.await {
        Ok(response) => Ok(response),
        Err(_) => Ok(error_response.unwrap()),
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

/// Resolves a request URI to the appropriate PHP file and path info
/// Returns PhpRoute with script_name and path_info
pub fn resolve_php_index(document_root: &Path, request_uri: &str) -> PhpRoute {
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

  // Try to find index.php by traversing from most specific to least specific path
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
  use super::*;
  use std::fs;

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
