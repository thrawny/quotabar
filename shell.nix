{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
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

    # GLib and friends (pulled in by gtk4, but explicit for clarity)
    glib
    cairo
    pango
    gdk-pixbuf
    graphene
  ];

  # Set up environment for linking
  shellHook = ''
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [
      pkgs.gtk4
      pkgs.gtk4-layer-shell
      pkgs.glib
    ]}:$LD_LIBRARY_PATH"
  '';
}
