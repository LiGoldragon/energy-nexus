{
  description = "Energy Nexus version 1";
  inputs = {
    tests = { url = "github:LiGoldragon/energy-nexus-tests/eeeca78c128d3a813762c50dbb14941df8bb9371"; flake = false; };
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
        mkdir -p "$out/crates/wire/tests"
        cp ${tests}/scenarios/admission/socket.rs "$out/crates/wire/tests/admission.rs"
        cp ${tests}/scenarios/persistence/local.rs "$out/crates/wire/tests/persistence.rs"
        cp ${tests}/scenarios/admission/legacy-emission.ethos "$out/crates/wire/tests/legacy-emission.ethos"
        cp -R ${tests}/fixtures/legacy-contracts "$out/crates/wire/tests/legacy-contracts"
      '';
      calculationArtifacts = rust.craneLib.buildDepsOnly {
        src = source;
        cargoExtraArgs = "-p energy-nexus-calculations --all-features";
      };
      admissionArtifacts = rust.craneLib.buildDepsOnly {
        src = source;
        cargoExtraArgs = "-p energy-nexus-wire";
      };
      wireCheck = rust.craneLib.cargoTest {
        src = source;
        cargoArtifacts = admissionArtifacts;
        cargoTestExtraArgs = "-p energy-nexus-wire";
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
      checks.admission = wireCheck;
      checks.persistence = wireCheck;
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
