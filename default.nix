{ pkgs ? import <nixpkgs> {} }:
let
  # 1. Fetch Crane
  # A library specifically designed to build Rust on Nix efficiently
  craneSrc = builtins.fetchTarball "https://github.com/ipetkov/crane/archive/master.tar.gz";
  craneLib = import craneSrc { inherit pkgs; };
  
  # Tell Crane to use the unstable Rust toolchain
  # craneLibUnstable = craneLib.overrideToolchain unstable.rustc;

  # 3. Clean the Source
  # This filters out garbage (git history, target folder, READMEs) so
  # Nix doesn't rebuild just because you fixed a typo in the README.
  src = craneLib.cleanCargoSource ./.;

  # 4. Common Arguments
  # These are inputs used for BOTH the dependency build and the final build.
  commonArgs = {
    inherit src;
    strictDeps = true;

    # Tools needed at compile time (pkg-config is CRITICAL for finding libopus)
    nativeBuildInputs = [ 
      pkgs.pkg-config 
      pkgs.makeWrapper
    ];

    # Libraries linked at runtime
    buildInputs = [ 
      pkgs.libopus 
      pkgs.openssl 
    ];
  };

  # 5. STEP ONE: Build Dependencies Only (The Cache Layer)
  # This compiles Cargo.toml + Cargo.lock.
  # If you change src/main.rs, this step is SKIPPED (cached).
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

in
# 6. STEP TWO: Build the Application
# Takes the artifacts from Step 1 and compiles your actual source code.
craneLib.buildPackage (commonArgs // {
  inherit cargoArtifacts;
  
  pname = "spotbot";
  version = "0.1.0";

  # Optional: Disable tests if they try to access the network (which fails in Nix)
  doCheck = false; 

  runtimePrograms = [
    pkgs.yt-dlp
    pkgs.ffmpeg
  ];

  postInstall = ''
    wrapProgram $out/bin/parrot \
        --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.ffmpeg pkgs.yt-dlp ]}
  '';
})
