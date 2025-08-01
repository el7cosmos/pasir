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
  // sender: Sender<PhpJob>,
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
  // type Response = Response<Channel<Bytes, Error>>;
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
      let (script_name, path_info) = resolve_php_index(root.as_path(), head.uri.path());
      let error_response = Response::internal_server_error(UnsyncBoxBody::default());

      let context = Context::new(
        root,
        script_name,
        path_info,
        local_addr,
        peer_addr,
        head,
        bytes,
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

/// Resolves a request URI to the appropriate index.php file and path info
/// Returns (index_php_path, path_info)
pub fn resolve_php_index(document_root: &Path, request_uri: &str) -> (String, Option<String>) {
  // Clean the request URI (remove query string, normalize slashes)
  let clean_uri = request_uri.split('?').next().unwrap_or(request_uri);
  let clean_uri = clean_uri.trim_start_matches('/').trim_end_matches('/');

  // Split the path into segments
  let segments: Vec<&str> = if clean_uri.is_empty() {
    vec![]
  } else {
    clean_uri.split('/').filter(|s| !s.is_empty()).collect()
  };

  // Check if the URI explicitly contains index.php
  if let Some(index_php_pos) = segments.iter().position(|&s| s == "index.php") {
    // URI contains index.php explicitly
    let script_path_segments = &segments[0..index_php_pos];
    let path_info_segments = &segments[index_php_pos + 1..];

    // Build the script path
    let script_path = if script_path_segments.is_empty() {
      "/index.php".to_string()
    } else {
      format!("/{}/index.php", script_path_segments.join("/"))
    };

    // Build path info
    let path_info = if path_info_segments.is_empty() {
      None
    } else {
      Some(format!("/{}", path_info_segments.join("/")))
    };

    // Verify the index.php file actually exists
    let mut file_path = document_root.to_path_buf();
    for segment in script_path_segments {
      file_path.push(segment);
    }
    file_path.push("index.php");

    if file_path.exists() && file_path.is_file() {
      return (script_path, path_info);
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

      return (relative_index_path, path_info);
    }
  }

  // Fallback to root index.php if it exists
  let root_index = document_root.join("index.php");
  if root_index.exists() && root_index.is_file() {
    let path_info = if clean_uri.is_empty() { None } else { Some(format!("/{clean_uri}")) };
    ("/index.php".to_string(), path_info)
  } else {
    // No index.php found anywhere
    ("/index.php".to_string(), Some(format!("/{clean_uri}")))
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

    // Create index.php files
    fs::write(temp_dir.join("index.php"), "<?php echo 'root'; ?>").unwrap();
    fs::write(temp_dir.join("some_path/index.php"), "<?php echo 'some_path'; ?>").unwrap();

    // Test cases
    let test_cases = vec![
      ("/", "/index.php", None),
      ("/some_path/admin", "/some_path/index.php", Some("/admin".to_string())),
      ("/some_other_path/admin", "/index.php", Some("/some_other_path/admin".to_string())),
      ("/some_path/", "/some_path/index.php", None),
      ("/some_path", "/some_path/index.php", None),
      ("/some_path/index.php/admin", "/some_path/index.php", Some("/admin".to_string())),
    ];

    for (uri, expected_script, expected_path_info) in test_cases {
      let (script, path_info) = resolve_php_index(&temp_dir, uri);
      assert_eq!(script, expected_script, "Failed for URI: {}", uri);
      assert_eq!(path_info, expected_path_info, "Failed for URI: {}", uri);
    }

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
  }
}
