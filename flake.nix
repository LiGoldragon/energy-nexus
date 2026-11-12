{
  description = "Energy Nexus version 1";
  inputs = {
    tests = { url = "github:LiGoldragon/energy-nexus-tests/93cfe1a3d7e96c4ba43f0a4c3385fe95bfb23aea"; flake = false; };
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-build = { url = "github:LiGoldragon/rust-build"; inputs.nixpkgs.follows = "nixpkgs"; };
  };
  outputs = { nixpkgs, flake-utils, rust-build, tests, ... }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs { inherit system; };
      rust = rust-build.lib.${system}.fromPkgs pkgs;
      source = pkgs.runCommand "energy-stage-one-test-source" {} ''
        cp -R ${./.}/. "$out"
        chmod -R u+w "$out"
        mkdir -p "$out/crates/calculations/tests"
        cp ${tests}/scenarios/calculations/pure.rs "$out/crates/calculations/tests/pure.rs"
      '';
      calculationArtifacts = rust.craneLib.buildDepsOnly {
        src = source;
        cargoExtraArgs = "-p energy-nexus-calculations --all-features";
      };
      cargoArtifacts = rust.craneLib.buildDepsOnly { src = source; };
      package = rust.craneLib.buildPackage { src = source; inherit cargoArtifacts; cargoExtraArgs = "--workspace"; };
    in {
      packages.default = package;
      checks.calculations = rust.craneLib.cargoTest {
        src = source;
        cargoArtifacts = calculationArtifacts;
        cargoTestExtraArgs = "-p energy-nexus-calculations --all-features";
      };
      checks.calculations-plain = rust.craneLib.cargoTest {
        src = source;
        cargoArtifacts = calculationArtifacts;
        cargoTestExtraArgs = "-p energy-nexus-calculations --no-default-features";
      };
      checks.test = rust.craneLib.cargoTest { src = source; inherit cargoArtifacts; cargoTestExtraArgs = "--workspace -- --nocapture"; };
      checks.nexus-dependency-boundary = rust.craneLib.mkCargoDerivation {
        pname = "energy-nexus-dependency-boundary"; version = "0.1.0"; src = source; inherit cargoArtifacts;
        buildPhaseCargoCommand = ''
          cargo tree --locked -p energy-nexus-daemon -e normal > daemon-tree
          if grep -E 'datom-codec|serde_json|protos' daemon-tree; then
            echo "text or JSON dependency reached the Nexus process" >&2
            exit 1
          fi
        '';
        installPhaseCommand = ''mkdir -p "$out"; cp daemon-tree "$out/daemon-tree.txt"'';
      };
    });
}
