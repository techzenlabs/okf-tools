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
          # The fixture tenant's pins, written out rather than read from
          # fixtures/site/tenant/site.toml: the rev beside a source is the
          # *claim* okf-assemble --pinned verifies against the manifest, and
          # deriving the claim from the manifest would compare the file with
          # itself and could never fail. A real tenant's claim comes from the
          # generated nix/bundles.nix the same way — a second place that can
          # drift, which is exactly what the verification is for.
          alphaRev = "1111111111111111111111111111111111111111";
          betaRev = "2222222222222222222222222222222222222222";
          # The actual build script of a packages.site derivation over the
          # fixture tenant, extracted so site-must-fail can run it against
          # broken sources and watch it refuse. Because it is the produced
          # script and not a copy of the steps, a step deleted from
          # nix/site.nix disappears from what site-must-fail executes, the
          # broken build goes green, and the check goes red the same day.
          siteBuildScript = sources:
            (import ./nix/site.nix {
              inherit pkgs;
              okf = okf-tools;
              manifestDir = ./fixtures/site/tenant;
              inherit sources;
            }).buildCommand;
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
          section-collision =
            mkCheck "section-collision" ./nix/checks/section-collision.sh {};
          pinned-commit = mkCheck "pinned-commit" ./nix/checks/pinned-commit.sh {};
          scan-negative-control =
            mkCheck "scan-negative-control" ./nix/checks/scan-negative-control.sh {};
          # The gate §7.2 asked for: the scan reads the assembled `static/`
          # and `layouts/` trees, not `content/` alone. Asserted against a
          # fixture that plants a synthetic credential in `static/`, which
          # the content-only recipe cannot see.
          scan-site-root = mkCheck "scan-site-root" ./nix/checks/scan-site-root.sh {
            PLANTED_STATIC = ./fixtures/site/planted-static;
          };
          layout-fork = mkCheck "layout-fork" ./nix/checks/layout-fork.sh {};
          bundles-current =
            mkCheck "bundles-current" ./nix/checks/bundles-current.sh {};
          # `packages.site` for the synthetic two-bundle tenant, built by the
          # same nix/site.nix a real tenant's flake-module import uses — only
          # the sources differ (fixture directories here, `builtins.fetchGit`
          # over the generated nix/bundles.nix there), so the derivation a
          # tenant ships is the one exercised here. Building it runs all five
          # pipeline steps in the sandbox; the assertions then hold the
          # output to the okfSite contract: $SITE is the served root itself.
          packages-site =
            pkgs.runCommand "packages-site" {
              SITE = import ./nix/site.nix {
                inherit pkgs;
                okf = okf-tools;
                manifestDir = ./fixtures/site/tenant;
                sources = {
                  alpha = {
                    outPath = ./fixtures/site/alpha;
                    rev = alphaRev;
                  };
                  beta = {
                    outPath = ./fixtures/site/beta;
                    rev = betaRev;
                  };
                };
              };
            } ''
              fail() {
                echo "FAIL: $*" >&2
                exit 1
              }
              test -f "$SITE/index.html" || fail "the site root has no index.html"
              test -f "$SITE/404.html" || fail "the site has no 404 page"
              test -f "$SITE/build-lock.json" || fail "no build provenance was published"
              test -d "$SITE/pagefind" || fail "pagefind wrote no search index"
              test -f "$SITE/alpha/runbooks/relay-restart/index.html" ||
                fail "a bundle page did not render"
              # Sources verified at the manifest's own rev are the pinned
              # corpus, and calling them a local override was the whole
              # difference between the nix-built site and the CI-built one —
              # 83 pages of footer stamp on techzen, and pagefind fragments
              # proving the stamp sat inside indexed content. Nothing in a
              # pinned build may carry it; -r covers the pagefind index too.
              # The other direction — the stamp still firing for a genuine
              # working-tree override — is site-pipeline's assertion.
              if grep -rq "local build:" "$SITE"; then
                grep -rl "local build:" "$SITE" >&2
                fail "a verified pin was stamped as a local build"
              fi
              # The span, not the bare word: the stylesheet ships an
              # `.okf-local` rule on every build, stamped or not.
              if grep -rq 'class="okf-local"' "$SITE"; then
                grep -rl 'class="okf-local"' "$SITE" | head >&2
                fail "the local-build span reached a pinned build's pages"
              fi
              touch $out
            '';
          # The proof the pipeline's gates can still refuse: the same build
          # script packages.site runs, over three broken fixture tenants,
          # each asserted to fail at the right step for the right reason.
          site-must-fail = mkCheck "site-must-fail" ./nix/checks/site-must-fail.sh {
            PLANTED_SCRIPT = pkgs.writeText "planted-site-build" (siteBuildScript {
              alpha = {
                outPath = ./fixtures/site/alpha-planted;
                rev = alphaRev;
              };
              beta = {
                outPath = ./fixtures/site/beta;
                rev = betaRev;
              };
            });
            DRAFT_SCRIPT = pkgs.writeText "draft-site-build" (siteBuildScript {
              alpha = {
                outPath = ./fixtures/site/alpha-draft;
                rev = alphaRev;
              };
              beta = {
                outPath = ./fixtures/site/beta;
                rev = betaRev;
              };
            });
            DRIFT_SCRIPT = pkgs.writeText "drift-site-build" (siteBuildScript {
              alpha = {
                outPath = ./fixtures/site/alpha;
                rev = "ffffffffffffffffffffffffffffffffffffffff";
              };
              beta = {
                outPath = ./fixtures/site/beta;
                rev = betaRev;
              };
            });
            PLANTED_BUNDLE = ./fixtures/site/alpha-planted;
            DRAFT_BUNDLE = ./fixtures/site/alpha-draft;
            TENANT = ./fixtures/site/tenant;
          };
          # This repository is public, so it scans itself.
          self-scan = mkCheck "self-scan" ./nix/checks/self-scan.sh {
            SOURCE = ./.;
          };

          # The workflow is the one file here that cannot be verified by
          # running it: hosted Actions on this org refuse to start over
          # billing, and the self-hosted fleet has not allowlisted this
          # repository. A static check is the only verification available, so
          # it is wired in rather than run once by hand — the same reasoning,
          # and the same derivation, as in all four tenant repositories.
          workflow =
            pkgs.runCommand "workflow" {
              nativeBuildInputs = [pkgs.actionlint];
            } ''
              cd ${self}
              # Named explicitly, both of them. actionlint discovers workflows
              # and its config by walking up to a git root, and a store path
              # has neither.
              actionlint -config-file .github/actionlint.yaml .github/workflows/check.yml
              touch $out
            '';
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
