use crate::cli::Executable;
use ext_php_rs::ffi::ZEND_RESULT_CODE_FAILURE;
use ext_php_rs::ffi::ZEND_RESULT_CODE_SUCCESS;
use ext_php_rs::ffi::module_registry;
use ext_php_rs::ffi::zend_extension;
use ext_php_rs::ffi::zend_extensions;
use ext_php_rs::ffi::zend_module_entry;
use ext_php_rs::zend::ExecutorGlobals;
use nu_ansi_term::Color;
use pasir::error::PhpError;
use std::borrow::Cow;
use std::ffi::CStr;
use std::io::Write;

pub struct Module {}

impl Module {
  unsafe fn print_modules() -> anyhow::Result<()> {
    if unsafe { ext_php_rs::ffi::php_request_startup() } == ZEND_RESULT_CODE_FAILURE {
      return Err(anyhow::anyhow!(PhpError::RequestStartupFailed));
    }

    let stdout = std::io::stdout();
    let mut handle = std::io::BufWriter::new(stdout);

    writeln!(handle, "{}", Color::Cyan.bold().paint("[PHP Modules]"))?;

    let registry_ptr = std::ptr::addr_of_mut!(module_registry);
    let mut modules = unsafe { (*registry_ptr).values() }
      .map(|value| {
        let entry = unsafe { value.ptr::<zend_module_entry>().unwrap() };
        let name = unsafe { CStr::from_ptr((*entry).name) };
        name.to_string_lossy()
      })
      .collect::<Vec<Cow<str>>>();
    modules.sort_by_key(|key| key.to_ascii_lowercase());
    for value in modules {
      writeln!(handle, "{}", value)?;
    }

    writeln!(handle, "\n{}", Color::Cyan.bold().paint("[Zend Modules]"))?;

    let extensions_ptr = std::ptr::addr_of_mut!(zend_extensions);
    let mut extensions = unsafe { (*extensions_ptr).iter() }
      .map(|zend_extension: &zend_extension| {
        let name = unsafe { CStr::from_ptr(zend_extension.name) };
        name.to_string_lossy()
      })
      .collect::<Vec<Cow<str>>>();
    extensions.sort_by_key(|key| key.to_ascii_lowercase());
    for extension in extensions {
      writeln!(handle, "{}", extension)?;
    }

    ExecutorGlobals::get_mut().exit_status = ZEND_RESULT_CODE_SUCCESS;
    unsafe { ext_php_rs::ffi::php_output_end_all() }

    Ok(())
  }
}

impl Executable for Module {
  async fn execute(self) -> anyhow::Result<()> {
    unsafe { Self::print_modules() }
  }
}
