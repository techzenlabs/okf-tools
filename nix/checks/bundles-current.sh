# site.toml is the single source of a bundle's pin, and nix/bundles.nix is
# generated from it. A hand-edited pin in the generated file is a build that
# disagrees with the reviewed manifest — the same reproducibility failure as
# a forked layout — so the freshness gate has to go red the day it happens,
# and this check watches it do so before believing it.
#
# The check derivation tenants run is `okf-assemble --bundles --check` against
# their own tracked tree; this fixture exercises every verdict that command
# can reach.
set -euo pipefail

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

site="$TMPDIR/site"
mkdir -p "$site"
cd "$site"
rev1=$(printf '1%.0s' $(seq 40))
rev2=$(printf '2%.0s' $(seq 40))
rev3=$(printf '3%.0s' $(seq 40))
{
  echo 'schema_version = 1'
  echo 'tenant = "pinned"'
  echo '[site]'
  echo 'title = "Pinned"'
  echo 'base_url = "https://pinned.invalid/"'
  echo '[[bundle]]'
  echo 'id = "alpha"'
  echo 'repo = "https://forge.invalid/owner/alpha.git"'
  echo 'ref = "refs/heads/main"'
  echo "rev = \"$rev1\""
  echo 'fetch = "git+ssh"'
  echo '[[bundle]]'
  echo 'id = "beta"'
  echo 'repo = "https://forge.invalid/owner/beta.git"'
  echo 'ref = "refs/heads/main"'
  echo "rev = \"$rev2\""
  echo 'subdir = "docs"'
  echo 'fetch = "git+ssh"'
} > site.toml

# Generation is pure text over the manifest. This sandbox has no network, so
# the file appearing at all is the proof that writing it fetches nothing.
okf-assemble --bundles
test -f nix/bundles.nix || fail "nix/bundles.nix was not written"
grep -q 'url = "ssh://git@forge.invalid/owner/alpha.git";' nix/bundles.nix ||
  fail "the ssh URL was not derived from the https repo"
grep -q "rev = \"$rev1\";" nix/bundles.nix ||
  fail "the pinned rev did not reach the generated file"
if grep -q 'subdir' nix/bundles.nix; then
  fail "subdir leaked into the generated file; site.toml owns it"
fi

# Missing while the manifest opts in: the state a tenant is in before the
# first commit of the file, and it has to be named rather than passed.
mv nix/bundles.nix "$TMPDIR/held.nix"
if okf-assemble --bundles --check > missing.log 2>&1; then
  cat missing.log
  fail "a missing nix/bundles.nix passed the freshness check"
fi
grep -q "is missing" missing.log || fail "the missing file was not named"
mv "$TMPDIR/held.nix" nix/bundles.nix

okf-assemble --bundles --check > fresh.log 2>&1 ||
  { cat fresh.log; fail "a freshly generated file failed its own check"; }

# The negative control: a hand-edited pin. Assert the break really landed, by
# diffing the file, before believing the red.
cp nix/bundles.nix "$TMPDIR/before.nix"
sed -i "s/$rev1/$rev3/" nix/bundles.nix
if cmp -s "$TMPDIR/before.nix" nix/bundles.nix; then
  fail "the hand-edit did not land"
fi
if okf-assemble --bundles --check > drift.log 2>&1; then
  cat drift.log
  fail "a hand-edited pin passed the freshness check"
fi
cat drift.log
grep -q "does not match site.toml" drift.log || fail "the refusal did not say why"
grep -q "$rev3" drift.log || fail "the refusal did not name the drifted line"

# Regeneration heals it, which is the fix the message names.
okf-assemble --bundles
okf-assemble --bundles --check ||
  fail "regeneration did not restore agreement"

# A generated file the manifest no longer explains is stale, not
# grandfathered — and regeneration removes it rather than leaving it to rot.
sed -i '/^fetch = "git+ssh"$/d' site.toml
if okf-assemble --bundles --check > stale.log 2>&1; then
  cat stale.log
  fail "a bundles.nix no manifest explains passed the check"
fi
grep -q "no bundle in site.toml sets fetch" stale.log ||
  fail "the stale file was not explained"
okf-assemble --bundles
test ! -e nix/bundles.nix ||
  fail "the stale generated file survived regeneration"

# The refusals live in the manifest, where the message names the bundle: a
# scheme this build does not know, a repo the ssh URL cannot be derived from,
# and a manifest where only some bundles opt in.
sed -i 's|^id = "alpha"|id = "alpha"\nfetch = "git+hg"|' site.toml
if okf-assemble --bundles > scheme.log 2>&1; then
  cat scheme.log
  fail "an unknown fetch scheme was accepted"
fi
grep -q 'git+ssh' scheme.log || fail "the refusal did not name the one supported scheme"

sed -i 's|^fetch = "git+hg"$|fetch = "git+ssh"|' site.toml
sed -i 's|^repo = "https://forge.invalid/owner/alpha.git"|repo = "file:///owner/alpha"|' site.toml
if okf-assemble --bundles > underivable.log 2>&1; then
  cat underivable.log
  fail "a file:// repo with fetch = git+ssh was accepted"
fi
grep -q 'https://' underivable.log || fail "the refusal did not say what it needs"

sed -i 's|^repo = "file:///owner/alpha"|repo = "https://forge.invalid/owner/alpha.git"|' site.toml
if okf-assemble --bundles > mixed.log 2>&1; then
  cat mixed.log
  fail "a manifest where only one bundle opts in was accepted"
fi
grep -q 'together' mixed.log || fail "the refusal did not explain all-or-none"
