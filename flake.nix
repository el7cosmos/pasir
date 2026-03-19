{
  description = "Pasir - PHP Application Server In Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, crane, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        mkPhp = phpPkg:
          let
            phpWithFlags = phpPkg.override {
              embedSupport = true;
              ztsSupport = true;
              zendSignalsSupport = false;
              zendMaxExecutionTimersSupport = pkgs.stdenv.isLinux;
            };
          in phpWithFlags.unwrapped;

        phpVersions = {
          php82 = mkPhp pkgs.php82;
          php83 = mkPhp pkgs.php83;
          php84 = mkPhp pkgs.php84;
          php85 = mkPhp pkgs.php85;
        };

        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type)
            || (builtins.match ".*\\.h$" path != null)
            || (builtins.match ".*craft\\.yml$" path != null)
            || (builtins.match ".*/patches/.*" path != null)
            || (builtins.match ".*/patches$" path != null)
            || (builtins.match ".*llvm-config$" path != null)
            || (builtins.match ".*/build/.*" path != null)
            || (builtins.match ".*/build$" path != null);
        };

        mkPasir = php:
          let
            commonArgs = {
              inherit src;
              strictDeps = true;
              pname = "pasir";
              doCheck = false;

              nativeBuildInputs = [
                pkgs.llvmPackages.libclang
                pkgs.clang
                pkgs.pkg-config
              ];

              buildInputs = [ php php.dev ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
                pkgs.libiconv
                pkgs.darwin.apple_sdk.frameworks.Security
              ];

              LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
              BINDGEN_EXTRA_CLANG_ARGS = pkgs.lib.optionals pkgs.stdenv.isLinux
                (builtins.map (a: ''-isystem ${a}/include'') [
                  pkgs.stdenv.cc.cc
                  pkgs.glibc.dev
                ]);
              PHP = "${php}/bin/php";
              PHP_CONFIG = "${php.dev}/bin/php-config";
            };

            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          in
          craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
          });

        mkDevShell = php: craneLib.devShell {
          inputsFrom = [ (mkPasir php) ];

          packages = [
            pkgs.rust-analyzer
            pkgs.cargo-nextest
            php
          ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          PHP = "${php}/bin/php";
        };

      in
      {
        packages = {
          default = mkPasir phpVersions.php84;
          php82 = mkPasir phpVersions.php82;
          php83 = mkPasir phpVersions.php83;
          php84 = mkPasir phpVersions.php84;
          php85 = mkPasir phpVersions.php85;
        };

        devShells = {
          default = mkDevShell phpVersions.php84;
          php82 = mkDevShell phpVersions.php82;
          php83 = mkDevShell phpVersions.php83;
          php84 = mkDevShell phpVersions.php84;
          php85 = mkDevShell phpVersions.php85;
        };
      }
    );
}
