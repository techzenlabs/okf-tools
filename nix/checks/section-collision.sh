# A page and a directory listing that want one URL, watched being dropped
# before the gate that refuses them is believed.
#
# OKF §8 reserves `index.md` for a directory listing, and okf-assemble renames
# it to `_index.md` so Hugo reads the directory as a section rather than as a
# leaf bundle. That rename is also what makes the directory publish at its own
# URL, so a sibling `<name>.md` is a page wanting the same one. Hugo resolves
# it by publishing one of them, exit 0, no warning.
#
# Three assertions, in the order that makes each believable: Hugo loses the
# page and says nothing; okf-assemble refuses the shape and names both files;
# the two shapes that look like it and lose nothing stay silent.
set -euo pipefail
export HOME="$TMPDIR"
site="$TMPDIR/collision"
mkdir -p "$site"
cd "$site"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

cat > site.toml <<'TOML'
schema_version = 1
tenant = "collision"
[site]
title = "Collision"
base_url = "https://collision.invalid/"
[[bundle]]
id = "collision"
repo = "https://example.invalid/collision.git"
ref = "refs/heads/main"
rev = "4444444444444444444444444444444444444444"
subdir = "."
TOML

# 1. The refusal, and what it says. Both source paths and the URL, or an
#    author reads "differs" and goes looking for a byte mismatch.
if okf-assemble --local "collision=$FIXTURES/collision" > refused.log 2>&1; then
  fail "okf-assemble accepted a bundle whose plans.md and plans/index.md want one URL"
fi
cat refused.log
grep -q 'collision/plans.md and collision/plans/index.md' refused.log ||
  fail "the refusal did not name both source files"
grep -q '/collision/plans/' refused.log ||
  fail "the refusal did not name the URL the two contend for"
grep -q 'collision/Notes.md and collision/notes/index.md' refused.log ||
  fail "the refusal missed the pair whose names differ only after Hugo sanitises them"
test "$(grep -c 'both publish at /collision/' refused.log)" -eq 2 ||
  fail "expected exactly the two collisions the fixture carries"

# 2. Now take the collisions out and watch the same bundle assemble, build and
#    verify clean. The negative controls travel with it: guides.md sits beside
#    a guides/ that carries no listing, and standalone.md beside nothing.
tree="$TMPDIR/fixed"
cp -r "$FIXTURES/collision" "$tree"
chmod -R u+w "$tree"
mv "$tree/plans.md" "$tree/plans/overview.md"
mv "$tree/Notes.md" "$tree/notes/overview.md"
okf-assemble --local "collision=$tree"
hugo --logLevel warn > fixed.log 2>&1
grep -qiE '^(WARN|ERROR)' fixed.log && fail "hugo warned on the repaired tree"
test -f public/collision/plans/overview/index.html ||
  fail "the renamed page did not publish"
test -f public/collision/plans/index.html ||
  fail "the listing did not publish as a section"
test -f public/collision/guides/index.html ||
  fail "a page beside a directory with no listing did not publish"
test -f public/collision/guides/deeper/index.html ||
  fail "a page under a directory with no listing did not publish"
test -f public/collision/standalone/index.html ||
  fail "the negative control page did not publish"
okf-assemble --verify-raw

# 3. The measurement the gate rests on: put one collision back into the
#    assembled tree, behind okf-assemble's back, and watch Hugo drop a page
#    while reporting success.
cp content/collision/plans/overview.md content/collision/plans.md
rm -rf public
hugo --logLevel warn --printPathWarnings > dropped.log 2>&1 ||
  fail "hugo failed, which would make this loud rather than silent"
if grep -qiE '^(WARN|ERROR)' dropped.log; then
  fail "hugo warned about the collision, which would make this gate unnecessary"
fi
test ! -f public/collision/plans.html ||
  fail "the page published somewhere of its own after all"
grep -q 'what makes it publish as a section' public/collision/plans/index.md ||
  fail "the section listing is not what survived at /collision/plans/"
if grep -q 'cannot both be published' public/collision/plans/index.md; then
  fail "the page survived, so this run measured nothing"
fi
echo "hugo published the listing at /collision/plans/, dropped content/collision/plans.md, and exited 0"

# 4. A bundle that already carries an _index.md beside an index.md is refused
#    rather than renamed over. std::fs::rename replaces its destination, so
#    this used to be a second silent loss inside the assembler itself.
both="$TMPDIR/both"
cp -r "$tree" "$both"
chmod -R u+w "$both"
printf '# Already assembled\n' > "$both/plans/_index.md"
if okf-assemble --local "collision=$both" > clobber.log 2>&1; then
  fail "okf-assemble renamed index.md over an _index.md that was already there"
fi
cat clobber.log
grep -q 'carries both index.md and _index.md' clobber.log ||
  fail "the refusal did not say which directory holds both"
echo "every shape measured: one refused, one built, two silent, one clobber refused"
