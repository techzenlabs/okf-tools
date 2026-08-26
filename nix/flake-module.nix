# A flake-parts module a consuming repository imports to gain the OKF checks.
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
    okf = okfTools.packages.${system}.okf-tools;
  in {
    checks = {
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
    };
  };
}
