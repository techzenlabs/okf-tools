# "Do not fork the theme" turned into a gate, asserted from both sides.
#
# A tenant overlay composes through Hugo's own lookup and has to pass. A
# tracked copy of a shared layout has to fail, because a layout bug is
# invisible until somebody reads a page, and this estate has already paid for
# a frozen copy whose upstream fix had nowhere to go.
set -euo pipefail
export HOME="$TMPDIR"
export GIT_CONFIG_GLOBAL="$TMPDIR/gitconfig"
export GIT_CONFIG_NOSYSTEM=1
git config --global user.name "fixture"
git config --global user.email "fixture@example.invalid"
git config --global init.defaultBranch main

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

tenant="$TMPDIR/tenant"
mkdir -p "$tenant/layouts/partials" "$tenant/layouts/alpha" "$tenant/layouts/_default"
cd "$tenant"
git init --quiet
echo "brand" > layouts/partials/brand.html
echo "one bundle's own page template" > layouts/alpha/single.html
git add -A
git commit --quiet -m "a legitimate overlay"

okf-check --layouts > overlay.log 2>&1 || { cat overlay.log; fail "a legitimate overlay was refused"; }
cat overlay.log

# Now fork a file okf-tools owns.
echo "a private copy of the shell" > layouts/_default/baseof.html
git add -A
git commit --quiet -m "fork the shell"
if okf-check --layouts > forked.log 2>&1; then
  cat forked.log
  fail "a tracked copy of baseof.html was not reported"
fi
cat forked.log
grep -q "layouts/_default/baseof.html" forked.log || fail "the report did not name the forked file"

# The distinction the gate turns on: okf-assemble writes the shared set into
# the working tree on every build, and an untracked copy is not a fork.
git rm --quiet --cached layouts/_default/baseof.html
printf 'layouts/_default/baseof.html\n' > .gitignore
git add .gitignore
git commit --quiet -m "stop tracking it"
okf-check --layouts > untracked.log 2>&1 || { cat untracked.log; fail "an untracked build artefact was reported as a fork"; }
cat untracked.log
