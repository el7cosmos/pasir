#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

pub use ext_php_rs::ffi::*;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
