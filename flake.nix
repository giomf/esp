{
  description = "ESP Rust Flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        fhs = pkgs.buildFHSEnv {
          name = "fhs-shell";
          targetPkgs = pkgs: [
            # https://docs.espressif.com/projects/esp-idf/en/latest/esp32/get-started/linux-macos-setup.html#for-linux-users
            toolchain
            pkgs.gcc
            pkgs.pkg-config
            pkgs.libclang.lib
            pkgs.gnumake
            pkgs.cmake
            pkgs.ninja
            pkgs.git
            pkgs.wget
            pkgs.cargo-generate
            pkgs.espflash
            pkgs.python3
            pkgs.python3Packages.pip
            pkgs.python3Packages.virtualenv
            pkgs.ldproxy
            pkgs.zlib
            pkgs.libxml2
          ];
          runScript = "fish";
        };
      in
      {
        devShells.default = fhs.env;
      }
    );
}
