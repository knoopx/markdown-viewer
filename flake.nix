{
  description = "markdown-viewer — GPUI markdown viewer (one window per file)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
      };

      guiDeps = with pkgs; [

        libxcb
        libX11
        libxcursor
        libxkbcommon
        freetype
        fontconfig
        vulkan-headers
        vulkan-loader

        wayland
        libXext
        libgbm
        libGL

        gtk4
      ];

      vulkanEnv = {
        RUSTFLAGS = "-L ${pkgs.vulkan-loader}/lib";
      };

      desktopItem = pkgs.makeDesktopItem {
        name = "markdown-viewer";
        desktopName = "Markdown Viewer";
        exec = "markdown-viewer %f";
        icon = "text-markdown";
        terminal = false;
        type = "Application";
        mimeTypes = [ "text/markdown" ];
      };
    in
    {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "markdown-viewer";
        version = "0.1.0";
        src = ./.;
        cargoLock = {
          lockFile = ./Cargo.lock;
        };
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = guiDeps;
        env = vulkanEnv;
        postInstall = ''
          mkdir -p $out/share/applications
          install -m 444 ${desktopItem}/share/applications/markdown-viewer.desktop \
            $out/share/applications/markdown-viewer.desktop
        '';
      };
      defaultPackage.${system} = self.packages.${system}.default;

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          pkg-config
          gcc.cc
          clang
        ] ++ guiDeps;
        env = vulkanEnv // {

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
