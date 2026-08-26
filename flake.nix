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
        okf-tools-unwrapped = pkgs.rustPlatform.buildRustPackage {
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
        # `okf-assemble` shells out to git, tar and zstd, and copies the
        # mermaid bundle out of a store path. Wrapping is what makes a
        # consumer's install one thing to pin rather than a tool plus a list
        # of things that have to already be on PATH.
        okf-tools = pkgs.symlinkJoin {
          name = "okf-tools";
          paths = [okf-tools-unwrapped];
          nativeBuildInputs = [pkgs.makeWrapper];
          postBuild = ''
            wrapProgram $out/bin/okf-assemble \
              --set-default OKF_MERMAID_JS ${mermaidJs} \
              --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.gitMinimal pkgs.gnutar pkgs.zstd]}
          '';
          inherit (okf-tools-unwrapped) meta;
        };

        # The browser bundle, copied and never executed. It is pinned by
        # flake.lock with everything else and runs only in a reader's browser,
        # which is what keeps the build's trust surface to two binaries plus
        # templates written here.
        #
        # Extracted into a derivation of its own rather than referenced inside
        # mermaid-cli's, because mermaid-cli carries chromium: referencing the
        # store path directly puts a browser in the runtime closure of every
        # tenant's tool, and of every NixOS host a site is copied to. The one
        # file has no runtime references at all.
        mermaidJs =
          pkgs.runCommand "mermaid.min.js" {
            passthru.upstream = pkgs.mermaid-cli;
          } ''
            install -Dm444 \
              ${pkgs.mermaid-cli}/lib/node_modules/@mermaid-js/mermaid-cli/node_modules/mermaid/dist/mermaid.min.js \
              $out
          '';

        # Every gate below is a fixture for a failure that otherwise produces a
        # *successful build with wrong output*. Each one was watched failing
        # before it was believed, and each builds by name:
        # `nix build .#checks.<system>.<name>`.
        siteChecks = let
          mkCheck = name: script: extra:
            pkgs.runCommand name ({
                nativeBuildInputs = [
                  okf-tools
                  pkgs.hugo
                  pkgs.pagefind
                  pkgs.git
                  pkgs.python3
                ];
                FIXTURES = ./fixtures;
              }
              // extra) ''
              bash ${script}
              touch $out
            '';
        in {
          site-pipeline = mkCheck "site-pipeline" ./nix/checks/site-pipeline.sh {
            ASSERTIONS = ./nix/checks/site-assertions.py;
          };
          leaf-bundle-rename =
            mkCheck "leaf-bundle-rename" ./nix/checks/leaf-bundle-rename.sh {};
          pinned-commit = mkCheck "pinned-commit" ./nix/checks/pinned-commit.sh {};
          scan-negative-control =
            mkCheck "scan-negative-control" ./nix/checks/scan-negative-control.sh {};
          layout-fork = mkCheck "layout-fork" ./nix/checks/layout-fork.sh {};
          # This repository is public, so it scans itself.
          self-scan = mkCheck "self-scan" ./nix/checks/self-scan.sh {
            SOURCE = ./.;
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
            # The site half, so `just build` and `just serve` work in a
            # checkout of this repository as well as in a tenant's.
            pkgs.hugo
            pkgs.pagefind
            pkgs.just
            pkgs.zstd
          ];
          # okf-assemble copies this rather than running it.
          OKF_MERMAID_JS = mermaidJs;
        };

        checks =
          {
            # The fixtures live in the crate's own tests, and building the
            # package runs them, so this check is the regression suite.
            okf-tools = okf-tools-unwrapped;
          }
          // siteChecks;
      };
    };
}
