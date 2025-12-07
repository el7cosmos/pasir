use pasir_build::php_info::PHPInfo;

fn main() -> anyhow::Result<()> {
  let php = pasir_build::find_executable("php", "PHP")?;
  let info = PHPInfo::get(&php)?;

  pasir_build::api_version::check_php_version(&info)?;

  Ok(())
}
