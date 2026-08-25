{
  description = "okf-tools: Open Knowledge Format v0.2 conformance, indexing and site assembly";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs @ {
    self,
    nixpkgs,
    flake-parts,
  }:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];

      # Consumers import this to gain checks.okf-conformance and
      # checks.okf-index-current without copying twenty lines per repo.
      flake.flakeModules.checks = import ./nix/flake-module.nix {okfTools = self;};

      perSystem = {
        pkgs,
        system,
        ...
      }: let
        okf-tools = pkgs.rustPlatform.buildRustPackage {
          pname = "okf-tools";
          version = "0.1.0";
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            # fixtures/ is the regression corpus and must reach the build;
            # result/ and .git/ must not.
            filter = path: type: let
              base = baseNameOf path;
            in
              base != "result" && base != ".git" && base != "target";
          };
          cargoLock.lockFile = ./Cargo.lock;
          # The parity fixtures are the whole point of the port; run them.
          doCheck = true;
          meta = {
            description = "OKF v0.2 conformance checker, index generator and staleness report";
            mainProgram = "okf-check";
          };
        };
      in {
        packages.okf-tools = okf-tools;
        packages.default = okf-tools;

        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rust-analyzer
            pkgs.python3 # the parity reference runs the original scripts
          ];
        };

        # Building the package runs the test suite, which is where the
        # fixtures live, so this check is the regression suite.
        checks.okf-tools = okf-tools;
      };
    };
}
