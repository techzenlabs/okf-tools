# The site build behind `packages.<system>.site`: `just build` with step zero
# deleted. The five steps are the justfile's five, in order —
#
#   1. `okf-assemble --pinned <id>=<store path>@<rev>` for every bundle: the
#      fetch disappears because nix already put the sources in the store, and
#      the rev each source was fetched at is verified against site.toml's own
#      pin — a drifted fetch specification fails the build here rather than
#      shipping the wrong corpus under a lock file that claims the manifest's
#      revs. A verified pin is the pinned corpus, so no page is stamped
#      "local build"; that stamp is `--local`'s, for a working tree standing
#      in for the pin on somebody's laptop.
#   2. `okf-scan .` over the assembled site root with `public/` excluded and
#      the caller's `--deny` list, which is the justfile's step 2 exactly:
#      `content` alone never reads `static/` or `layouts/`, and those are the
#      shared assets that publish;
#   3. `hugo`;
#   4. `okf-assemble --verify-raw`;
#   5. `pagefind --site public`.
#
# Every gate the pipeline has today still runs, in the sandbox, on every
# `nix build`. Nothing here has network access, which is the proof that no
# step fetches anything. The checks.site-must-fail derivation runs this same
# script against broken fixtures and asserts steps 1, 2 and 4 go red for the
# right reason — a step deleted from this file fails that check the same day.
#
# This file is a plain function rather than part of the flake module so the
# same derivation builds a real tenant (sources fetched at evaluation from
# the generated `nix/bundles.nix`) and this repository's own fixture tenant
# (sources are fixture directories) — the check and the product cannot
# diverge if they are the same code.
#
# `sources` maps bundle id to something carrying the bundle repository's
# *root* and the commit it was fetched at: a `builtins.fetchGit` result as it
# stands, or `{ outPath; rev; }` for a fixture. The rev is required — it is
# the claim `okf-assemble --pinned` verifies, so a source that cannot say
# what it was fetched at is refused at evaluation. `subdir` is applied by
# `okf-assemble` from site.toml, which is the one place it lives.
#
# The output is the rendered site root itself — `public/`'s contents — which
# is the contract `dotfiles`' okfSite module already reads:
# `root = input.packages.${system}.site`.
# `denyList` is a path to the caller's `okf-scan --deny` file: one term per
# line, the other tenants and their clients and hosts. It is an argument and
# not a file in any repository, because three of the four site repositories
# are controlled by a client and a tracked roster is the disclosure the list
# exists to prevent. A tenant supplies it from the machine that runs the
# build. Omitting it is not a quiet degradation: `okf-assemble` refuses a
# build whose scan was not armed, so the derivation goes red naming the
# variable.
{
  pkgs,
  okf,
  manifestDir,
  sources,
  denyList ? null,
}: let
  inherit (pkgs) lib;
  manifest = builtins.fromTOML (builtins.readFile (manifestDir + "/site.toml"));
  wanted = map (bundle: bundle.id) manifest.bundle;
  unsourced = lib.filter (id: !(sources ? ${id})) wanted;
  surplus = lib.filter (id: !(lib.elem id wanted)) (builtins.attrNames sources);
  unrevved = lib.filter (id: (sources ? ${id}) && !(sources.${id} ? rev)) wanted;
  # Refused at evaluation rather than left to fail mid-build: a bundle with
  # no source would reach `okf-assemble`'s fetch path, and this sandbox has
  # no network, so the message would be git's rather than a useful one.
  checked =
    if unsourced != []
    then throw "packages.site: no source for bundle(s) ${lib.concatStringsSep ", " unsourced}; every bundle in site.toml needs one"
    else if surplus != []
    then throw "packages.site: source(s) ${lib.concatStringsSep ", " surplus} name no bundle in site.toml"
    else if unrevved != []
    then throw "packages.site: source(s) for ${lib.concatStringsSep ", " unrevved} carry no rev; pass the fetchGit result (or { outPath; rev; }) so okf-assemble can verify the pin"
    else sources;
  pinnedFlags =
    lib.concatMapStringsSep " "
    (id: "--pinned ${lib.escapeShellArg "${id}=${checked.${id}}@${checked.${id}.rev}"}")
    wanted;
  hasAllowFile = builtins.pathExists (manifestDir + "/credentials.allow");
in
  pkgs.runCommand "${manifest.tenant}-site" {
    nativeBuildInputs = [okf pkgs.hugo pkgs.pagefind];
  } ''
    export HOME="$TMPDIR"
    site="$TMPDIR/site"
    mkdir -p "$site"
    install -m 644 ${manifestDir}/site.toml "$site/site.toml"
    ${lib.optionalString hasAllowFile ''
      install -m 644 ${manifestDir}/credentials.allow "$site/credentials.allow"
    ''}
    cd "$site"
    ${lib.optionalString (denyList != null) ''
      export OKF_SCAN_DENY=${denyList}
    ''}
    okf-assemble ${pinnedFlags}
    okf-scan . --exclude public --deny "$OKF_SCAN_DENY"
    hugo
    okf-assemble --verify-raw
    pagefind --site public
    cp -r public "$out"
  ''
