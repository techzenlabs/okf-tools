# The rename that is mandatory rather than cosmetic, asserted from both sides.
#
# OKF §8 reserves `index.md` for a directory listing. Hugo reads a directory
# holding one as a *leaf bundle*, which demotes every sibling from a page to a
# page resource of it. The build succeeds either way and says nothing, so this
# fixture renders the same five files twice and asserts both counts.
set -euo pipefail
export HOME="$TMPDIR"
site="$TMPDIR/leaf"
mkdir -p "$site"
cd "$site"

cat > site.toml <<'TOML'
schema_version = 1
tenant = "leaf"
[site]
title = "Leaf"
base_url = "https://leaf.invalid/"
[[bundle]]
id = "leaf"
repo = "https://example.invalid/leaf.git"
ref = "refs/heads/main"
rev = "3333333333333333333333333333333333333333"
subdir = "."
TOML

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

okf-assemble --local "leaf=$FIXTURES/leafbundle"
hugo --logLevel warn > renamed.log 2>&1
renamed=$(find public/leaf -name index.html | wc -l)
renamed_raw=$(find public -name index.md | wc -l)
echo "with the rename: ${renamed} pages under the mount, ${renamed_raw} raw pages"
test "$renamed" -eq 5 ||
  fail "expected 5 pages under the mount after the rename, got ${renamed}"

# Put the filename back and watch the same five files collapse into one page,
# with the build still reporting success.
mv content/leaf/_index.md content/leaf/index.md
rm -rf public
hugo --logLevel warn > collapsed.log 2>&1 || fail "hugo failed, which would make this loud rather than silent"
collapsed=$(find public/leaf -name index.html | wc -l)
collapsed_raw=$(find public -name index.md | wc -l)
echo "as written:      ${collapsed} pages under the mount, ${collapsed_raw} raw pages"
test "$collapsed" -eq 1 ||
  fail "expected the leaf-bundle collision to leave 1 page, got ${collapsed}"
if grep -qiE '^(WARN|ERROR)' collapsed.log; then
  fail "hugo warned about the collision, which would make the rename optional"
fi
echo "five files render as ${renamed} pages renamed and ${collapsed} as written, and Hugo reports success either way"
