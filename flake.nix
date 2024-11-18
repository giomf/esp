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
        fhs = pkgs.buildFHSUserEnv {
          name = "fhs-shell";
          targetPkgs = pkgs: [
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
          ];
          runScript = "fish";
        };
      in
      {
        devShells.default = fhs.env;
        #   devShells.default = pkgs.mkShell {
        #     buildInputs = [
        #       # Rust toolchain
        #       toolchain

        #       pkgs.pkg-config
        #       pkgs.openssl

        #       # https://docs.espressif.com/projects/esp-idf/en/latest/esp32/get-started/linux-macos-setup.html#for-linux-users
        #       pkgs.wget
        #       pkgs.git
        #       pkgs.flex
        #       pkgs.bison
        #       pkgs.gperf
        #       pkgs.ldproxy
        #       pkgs.libffi
        #       pkgs.libusb1
        #       pkgs.python312Packages.python
        #       pkgs.python312Packages.pip
        #       pkgs.python312Packages.virtualenv
        #       pkgs.cmake
        #       pkgs.ninja
        #       pkgs.ccache
        #     ];
        #     env = {
        #       RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
        #     };
        #   };
      }
    );
}
