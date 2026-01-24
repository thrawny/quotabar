{
  description = "quotabar - Monitor API quota/usage for AI coding tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Rust toolchain
            rustc
            cargo
            rust-analyzer
            clippy
            rustfmt

            # Build dependencies
            pkg-config

            # GTK4 and layer-shell
            gtk4
            gtk4-layer-shell

            # Just for task running
            just
          ];
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "quotabar";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ gtk4 gtk4-layer-shell ];
        };
      }
    );
}
