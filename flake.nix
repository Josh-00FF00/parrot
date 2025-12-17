{
  description = "SpotBot";

  # 1. DEFINE YOUR INPUTS
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    
    # Crane is now a first-class input
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane, ... }:
    let
      # Support multiple architectures automatically
      system = "x86_64-linux"; 
      pkgs = nixpkgs.legacyPackages.${system};

      # Crane is already initialized for us
      craneLib = crane.mkLib pkgs;
      
      commonArgs = {
        src = craneLib.cleanCargoSource ./.;
        strictDeps = true;
        nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
        buildInputs = [ pkgs.libopus pkgs.openssl ];
      };

      cargoArtifacts = craneLib.buildDepsOnly commonArgs;
    in
    {
      # 3. THE PACKAGE
      # Run with: 'nix build'
      packages.${system}.default = craneLib.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        pname = "spotbot";
        version = "0.1.0";
        doCheck = false;
        
        postInstall = ''
          wrapProgram $out/bin/parrot \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.ffmpeg-full pkgs.yt-dlp ]}
        '';
      });

      # 4. THE DEV ENVIRONMENT (Bonus!)
      # Run with: 'nix develop'
      # This drops you into a shell with cargo, rustc, and libs pre-configured.
      devShells.${system}.default = craneLib.devShell {
          checks = self.checks.${system};
      };
    };
}
