use crate::Stream;
use hyper::Request;
use std::net::IpAddr;
use std::str::FromStr;

pub(crate) trait RequestExt {
  fn client_ip(&self) -> Option<IpAddr>;
}

impl<B> RequestExt for Request<B> {
  fn client_ip(&self) -> Option<IpAddr> {
    // Check X-Forwarded-For header first (for proxies)
    if let Some(xff) = self.headers().get("x-forwarded-for")
      && let Ok(xff_str) = xff.to_str()
    {
      // Take the first IP in the chain
      if let Some(first_ip) = xff_str.split(',').next()
        && let Ok(ip_address) = IpAddr::from_str(first_ip.trim())
      {
        return Some(ip_address);
      }
    }

    // Check X-Real-IP header
    if let Some(real_ip) = self.headers().get("x-real-ip")
      && let Ok(ip_str) = real_ip.to_str()
      && let Ok(ip_address) = IpAddr::from_str(ip_str)
    {
      return Some(ip_address);
    }

    // Fall back to connection peer address (if available)
    self.extensions().get::<Stream>().map(|stream| stream.peer_addr.ip())
  }
}
