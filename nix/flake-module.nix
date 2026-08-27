# A flake-parts module a consuming repository imports to gain the OKF checks
# and, for a site tenant whose bundles opt into a nix fetch, the rendered
# site itself as `packages.<system>.site`.
#
# The point of this file is that a repository adopting the standard adds one
# import and a config file, rather than the twenty-odd lines of `runCommand`
# each repository would otherwise copy — and then diverge from. Four repos in
# this estate already carry byte-identical copies of a script whose upstream
# fix never reached them, which is the failure this shape avoids.
#
# `okfTools` is okf-tools' own flake, captured where this file is imported.
# `self` inside the returned module is the *consumer's* flake, so the checks
# run against the consumer's source tree.
{okfTools}: {self, ...}: {
  perSystem = {
    pkgs,
    system,
    ...
  }: let
    inherit (pkgs) lib;
    okf = okfTools.packages.${system}.okf-tools;
    # A site tenant carries a manifest at its root; a knowledge repository
    # does not, and gets neither the bundles check nor `packages.site` — the
    # tool deliberately errors rather than false-greens without a manifest.
    hasManifest = builtins.pathExists (self + "/site.toml");
    # The generated fetch specification. `okf-assemble --bundles` writes it
    # from site.toml when the bundles opt into `fetch = "git+ssh"`, and the
    # okf-bundles-current check keeps it agreeing with the manifest, so this
    # input list is an artefact somebody can diff rather than a hand-written
    # copy of the pins.
    hasBundles = builtins.pathExists (self + "/nix/bundles.nix");
    # One store path per bundle, fetched at *evaluation* — through the
    # evaluating user's own ssh key, the way any private flake input is
    # fetched — so no secret ever enters a derivation and no host needs a
    # credential. `rev` pins it, which is what pure evaluation requires.
    #
    # `builtins.fetchGit` over an imported file rather than flake inputs,
    # because a flake's `inputs` must be a literal attrset and refuses
    # `import` at parse time (measured). The pin therefore never lands in
    # `flake.lock` at all: site.toml is the single source, and the generated
    # file is the only second place — gated by okf-bundles-current in the
    # tree, and verified again inside the build: each fetchGit result
    # carries the rev it was fetched at, site.nix hands that rev to
    # `okf-assemble --pinned`, and an assembly whose sources are not at
    # site.toml's own pins refuses. A drifted nix/bundles.nix therefore
    # cannot build a site at all, whether or not anyone ran the check.
    bundleSources =
      builtins.mapAttrs
      (_: bundle: builtins.fetchGit {inherit (bundle) url ref rev;})
      (import (self + "/nix/bundles.nix"));
  in {
    # The two knowledge-repository checks are gated on *not* being a site
    # tenant. A tenant repository authors no concept documents — its content
    # is assembled from bundle repositories at pinned revs, and each bundle's
    # conformance is enforced where the documents live — so `okf-check` at a
    # tenant root reads the tenant's own README and manifest as a
    # nonconformant bundle and can never pass. Measured on techzen-docs-site,
    # the first real consumer.
    checks = lib.optionalAttrs (!hasManifest) {
      # OKF v0.2 conformance (§11): parseable frontmatter and a non-empty
      # `type` on every concept document, §8/§9 structure for the reserved
      # index.md and log.md filenames.
      #
      # `${self}` is the consumer's whole source tree, so a `.gate-as-of` at
      # its root is an input to this derivation like every other tracked file.
      # That is what keeps the `stale_after` comparison honest under caching:
      # the day is data the repository commits, the cache invalidates when the
      # day moves, and no verdict here reads the build machine's clock. See
      # the README section on `stale_after`.
      okf-conformance =
        pkgs.runCommand "okf-conformance" {} ''
          cd ${self}
          ${okf}/bin/okf-check
          touch $out
        '';

      # Indexes are generated from concept frontmatter, so a stale index means
      # a document's title or description moved without its listing following.
      # This is the generated-artefact freshness gate: the tree disagreeing
      # with the tool that produced it is a reproducibility failure, not a
      # matter of taste, which is why it is an error rather than a warning.
      okf-index-current =
        pkgs.runCommand "okf-index-current" {} ''
          cd ${self}
          ${okf}/bin/okf-index --check
          touch $out
        '';
    }
    // lib.optionalAttrs hasManifest {
      # site.toml is the single source of a bundle's pin and nix/bundles.nix
      # is generated from it, so the tree disagreeing with the manifest is
      # the same reproducibility failure as a stale index: a hand-edited pin
      # goes red the day it happens. The check writes nothing and fetches
      # nothing, which is why it can run here with no network.
      okf-bundles-current =
        pkgs.runCommand "okf-bundles-current" {} ''
          cd ${self}
          ${okf}/bin/okf-assemble --bundles --check
          touch $out
        '';
    };

    # `just build` with step zero deleted: assemble from the store paths,
    # scan, hugo, verify-raw, pagefind — every gate the pipeline has today,
    # in the sandbox, on every `nix build`. Appears once the tenant commits
    # the generated nix/bundles.nix; until then the host-side okfSite module
    # keeps serving its placeholder.
    packages = lib.optionalAttrs (hasManifest && hasBundles) {
      site = import ./site.nix {
        inherit pkgs okf;
        manifestDir = self;
        sources = bundleSources;
      };
    };
  };
}
