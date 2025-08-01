use ext_php_rs::php_function;

#[php_function]
pub fn fastcgi_finish_request() -> bool {
  false
}
