variable "PASIR_VERSION" {
  type = tuple([number, number, number])
}
variable "PHP_VERSION" {
  type = tuple([number, number, number])
}
variable "RUST_VERSION" {
  type = tuple([number, number, number])
}

group "default" {
  targets = ["debian"]
}

target "debian" {
  name       = "debian-${variant}"
  dockerfile = "Dockerfile"
  matrix = {
    variant = ["trixie", "bookworm", "bullseye"]
  }
  args = {
    PHP_VERSION = join(".", PHP_VERSION)
    RUST_VERSION = join(".", RUST_VERSION)
    VARIANT = variant
  }
  tags = [
    "docker.io/el7cosmos/pasir:${PASIR_VERSION[0]}.${PASIR_VERSION[1]}.${PASIR_VERSION[2]}-php${PHP_VERSION[0]}.${PHP_VERSION[1]}.${PHP_VERSION[2]}-${variant}",
    "docker.io/el7cosmos/pasir:${PASIR_VERSION[0]}.${PASIR_VERSION[1]}.${PASIR_VERSION[2]}-php${PHP_VERSION[0]}.${PHP_VERSION[1]}-${variant}",
    "docker.io/el7cosmos/pasir:${PASIR_VERSION[0]}.${PASIR_VERSION[1]}-php${PHP_VERSION[0]}.${PHP_VERSION[1]}.${PHP_VERSION[2]}-${variant}",
    "docker.io/el7cosmos/pasir:${PASIR_VERSION[0]}.${PASIR_VERSION[1]}-php${PHP_VERSION[0]}.${PHP_VERSION[1]}-${variant}",
  ]
  labels = {
    "maintainer" = "Abi <el@elabee.me>"
  }
  platforms = ["linux/arm64", "linux/amd64"]
  context = "."
}
