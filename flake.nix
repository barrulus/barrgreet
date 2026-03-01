{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "barrgreet";
        version = "0.1.0";
        src = self;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
        buildInputs = with pkgs; [ wayland libxkbcommon vulkan-loader ];
        postInstall = ''
          wrapProgram $out/bin/barrgreet \
            --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [
              pkgs.wayland
              pkgs.libxkbcommon
              pkgs.vulkan-loader
            ]}
        '';
        meta.mainProgram = "barrgreet";
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [ cargo rustc pkg-config ];
        buildInputs = with pkgs; [ wayland libxkbcommon vulkan-loader ];
      };
    };
}
