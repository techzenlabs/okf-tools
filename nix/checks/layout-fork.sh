# "Do not fork the theme" turned into a gate, asserted from every side.
#
# A tenant overlay composes through Hugo's own lookup and has to pass. A
# tracked copy of a shared layout has to fail, because a layout bug is
# invisible until somebody reads a page, and this estate has already paid for
# a frozen copy whose upstream fix had nowhere to go.
#
# And a shared file the ignore file never heard of has to fail, one step
# earlier. `layouts/404.html` joined the set and four tenants' .gitignore
# files, all written before it existed, said nothing about it: four working
# trees carrying an untracked copy, one `git add -A` from the fork above.
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
# What a conformant tenant's ignore file holds, produced by the tool that
# owns the set rather than typed out here. Typing it out is the bug: a
# hand-written list is exactly what went stale when the set grew.
okf-check --shared-paths > .gitignore
git add -A
git commit --quiet -m "a legitimate overlay"

okf-check --layouts > overlay.log 2>&1 || { cat overlay.log; fail "a legitimate overlay was refused"; }
cat overlay.log

# Now fork a file okf-tools owns. `--force`, because the ignore file written
# above is what stops this happening by accident, and a real fork is somebody
# overriding it. The tenant flakes' own copy of this check forces for the same
# reason.
echo "a private copy of the shell" > layouts/_default/baseof.html
git add --force layouts/_default/baseof.html
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
git commit --quiet -m "stop tracking it"
okf-check --layouts > untracked.log 2>&1 || { cat untracked.log; fail "an untracked build artefact was reported as a fork"; }
cat untracked.log

# The gate one step earlier. Drop a single line from the ignore file — which
# is what an ignore file written before a shared file existed looks like —
# and the untracked copy has to be reported by name.
grep -v '^/layouts/404\.html$' .gitignore > .gitignore.next
mv .gitignore.next .gitignore
git add .gitignore
git commit --quiet -m "an ignore file older than the shared set"
if okf-check --layouts > unignored.log 2>&1; then
  cat unignored.log
  fail "a shared file the ignore file does not name was not reported"
fi
cat unignored.log
grep -q "layouts/404.html" unignored.log || fail "the report did not name the unignored file"
if grep -q "baseof" unignored.log; then
  fail "a file the ignore file does name was reported as well"
fi

# And the whole set restored passes, which is the state a tenant is in.
okf-check --shared-paths > .gitignore
git add .gitignore
git commit --quiet -m "name the whole set"
okf-check --layouts > restored.log 2>&1 || { cat restored.log; fail "a fully-ignored shared set was still reported"; }
cat restored.log
