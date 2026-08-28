# The fetch asks for a commit and never for a branch.
#
# A source repository is built here with two commits. The manifest pins the
# first, and the second adds a file. If the fetch resolved `ref` instead of
# `rev`, that file would be in the assembled tree — which is exactly what
# happens when a runner picks up whatever landed on main after the manifest
# was reviewed.
set -euo pipefail
export HOME="$TMPDIR"
# `okf-assemble` refuses a build whose scan was never armed with a deny
# list, so every check that assembles supplies the fixture tenant's. The
# file sits outside any tree these checks scan, which is the shape a real
# tenant's has: on the machine that runs the build, in no repository.
export OKF_SCAN_DENY="$FIXTURES/site/tenant/deny.list"
export GIT_CONFIG_GLOBAL="$TMPDIR/gitconfig"
export GIT_CONFIG_NOSYSTEM=1
git config --global user.name "fixture"
git config --global user.email "fixture@example.invalid"
git config --global init.defaultBranch main
git config --global uploadpack.allowAnySHA1InWant true

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

source="$TMPDIR/source"
cp -r "$FIXTURES/site/alpha" "$source"
chmod -R u+w "$source"
git -C "$source" init --quiet
git -C "$source" add -A
git -C "$source" commit --quiet -m "reviewed"
pinned=$(git -C "$source" rev-parse HEAD)

printf -- '---\ntype: "Reference"\ntitle: "Landed later"\n---\n\nNobody reviewed this.\n' \
  > "$source/landed-later.md"
git -C "$source" add -A
git -C "$source" commit --quiet -m "landed after the review"
later=$(git -C "$source" rev-parse HEAD)
test "$pinned" != "$later" || fail "the fixture did not produce two commits"

site="$TMPDIR/site"
mkdir -p "$site"
cd "$site"
{
  echo 'schema_version = 1'
  echo 'tenant = "pinned"'
  echo '[site]'
  echo 'title = "Pinned"'
  echo 'base_url = "https://pinned.invalid/"'
  echo '[[bundle]]'
  echo 'id = "alpha"'
  echo "repo = \"file://$source\""
  echo 'ref = "refs/heads/main"'
  echo "rev = \"$pinned\""
  echo 'subdir = "."'
} > site.toml

okf-assemble
test -f content/alpha/_index.md || fail "the pinned commit's content did not arrive"
if [ -e content/alpha/landed-later.md ]; then
  fail "content from a commit after the pinned one reached the build"
fi
grep -q "\"rev\": \"$pinned\"" static/build-lock.json ||
  fail "build-lock.json does not record the commit that was built"
echo "fetched ${pinned}, and the commit that landed after it (${later}) is absent"

# And the manifest refuses a moving ref where a commit belongs.
sed -i "s/rev = \"$pinned\"/rev = \"main\"/" site.toml
if okf-assemble > refused.log 2>&1; then
  fail "a branch name was accepted where a commit belongs"
fi
cat refused.log
grep -q "is not a 40-character commit" refused.log ||
  fail "the refusal did not say why"
