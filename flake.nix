{
  description = "parrot";

  # Nix will prompt users to accept these cache settings automatically
  nixConfig = {
    extra-substituters = [ "https://jawsh.cachix.org" ];
    extra-trusted-public-keys = [ "jawsh.cachix.org-1:QlM5u5Hnpc8A6nYPi676JCpmc6h3xEYXhCrFnzJTNQs=" ];
  };

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      nixpkgs,
      crane,
      ...
    }:
    let
      # Define the architectures you want to support
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      # Helper function to generate attributes for all systems
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          craneLib = crane.mkLib pkgs;

          commonArgs = {
            src = craneLib.cleanCargoSource ./.;
            strictDeps = true;
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.makeWrapper
            ];
            buildInputs = [
              pkgs.libopus
              pkgs.openssl
            ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          default = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "parrot";
              version = "1.6.0";
              doCheck = false;

              postInstall = ''
                wrapProgram $out/bin/parrot \
                  --prefix PATH : ${
                    pkgs.lib.makeBinPath [
                      pkgs.ffmpeg-full
                      pkgs.yt-dlp
                      pkgs.deno
                    ]
                  }
              '';
            }
          );
        }
      );

      # This drops you into a shell with cargo, rustc, and libs pre-configured.
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          craneLib = crane.mkLib pkgs;
        in
        {
          default = craneLib.devShell {
            # Make the same tools available in your local development environment
            packages = [
              pkgs.ffmpeg-full
              pkgs.yt-dlp
              pkgs.nodejs
            ];
          };
        }
      );
    };
}
