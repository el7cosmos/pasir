use actix_web::dev::RequestHead;
use actix_web::http::header::HeaderMap;
use std::ffi::c_void;

pub(crate) struct Context<'a> {
  pub(crate) request_head: &'a RequestHead,
  pub(crate) response_head: HeaderMap,
  pub(crate) response_body: &'a [u8],
  // pub(crate) response_builder: &'a mut HttpResponseBuilder,
}

impl<'a> Context<'a> {
  pub(crate) fn new(request_head: &'a RequestHead) -> Self {
    Context { request_head, response_head: HeaderMap::default(), response_body: &[] }
  }
  pub(crate) fn from_server_context(
    server_context: *mut c_void,
  ) -> Option<&'static mut Context<'a>> {
    unsafe { server_context.cast::<Self>().as_mut() }
  }
}
