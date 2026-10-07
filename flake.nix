{
  description = "markdown-viewer — GPUI markdown viewer (one window per file)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
      };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          # gpui (wgpu/winit) X11 + windowing + font stack.
          libxcb
          libX11
          libxcursor
          libxkbcommon
          freetype
          fontconfig
          vulkan-headers
          vulkan-loader
          # windowing + GL.
          wayland
          libXext
          libgbm
          libGL
          # native-theme-gtk links the GTK 4 runtime (pkg-config gtk4-1.0).
          gtk4
          # build tools.
          pkg-config
          gcc.cc
          clang
        ];
        env = {
          # final binary links -lvulkan (wgpu); point the linker at it.
          RUSTFLAGS = "-L ${pkgs.vulkan-loader}/lib";
          # wgpu/winit dlopen the wayland/X11 client libs at runtime (they are
          # NOT in the binary's RUNPATH), so every GUI client lib must be on
          # LD_LIBRARY_PATH for both the Wayland and X11 backends.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.vulkan-loader
            pkgs.wayland
            pkgs.libxkbcommon
            pkgs.libxcb
            pkgs.libX11
            pkgs.libxcursor
            pkgs.libXext
            pkgs.libgbm
            pkgs.libGL
            pkgs.gtk4
          ];
        };
      };
    };
}
