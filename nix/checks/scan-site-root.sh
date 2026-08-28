# The scan reads what the site publishes, not just what it was written from.
#
# For as long as the build recipe read `okf-scan content`, `static/` and
# `layouts/` were outside every gate in the estate — and those are the two
# trees the shared assets land in. Hugo byte-copies `static/` into `public/`,
# so a comment in a stylesheet is served to every reader of every tenant.
# That defect was live, on a real file, and nothing here would have caught it.
#
# So this check asserts three things in order, against a fixture that plants
# another tenant's name in `static/`:
#
#   1. `okf-scan content` — the recipe as it stood — passes over the planted
#      tree. That is the gap, and it is asserted rather than described.
#   2. The command `site/justfile` ships today fails over the same tree,
#      names the planted file and the rule, and does not reproduce the term.
#   3. The same command is clean once the planted file is gone, so it is a
#      gate and not a tripwire that fires on everything.
#
# The command in step 2 is read out of the *assembled* justfile rather than
# written here, so weakening the recipe changes what this check runs and the
# check goes red the same day. That is site-must-fail's principle applied to
# a recipe line instead of a build script.
#
# Then the other half of §7.2's gate: a build whose scan was never armed with
# a `--deny` list is refused, and refused on *supply* rather than on a file in
# the repository — a gate demanding a tracked roster would fail in every fresh
# clone and could only be satisfied by committing the other three tenants'
# names into a repository a client controls.
set -euo pipefail
export HOME="$TMPDIR"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

term=$(grep -v '^[[:space:]]*#' "$DENY_LIST" | grep -v '^[[:space:]]*$' | head -1)
test -n "$term" || fail "the deny list fixture holds no terms"
grep -q "$term" "$PLANTED_STATIC/css/tenant-brand.css" ||
  fail "the planted stylesheet no longer carries the denied term; the break did not land"

# Logs live outside the tree being scanned. A log inside it is a file the
# next scan reads, which moves the inspected-file count this check compares.
log="$TMPDIR/logs"
mkdir -p "$log"

site="$TMPDIR/site"
mkdir -p "$site"
install -m 644 "$FIXTURES/site/tenant/site.toml" "$site/site.toml"
install -m 644 "$FIXTURES/site/tenant/credentials.allow" "$site/credentials.allow"
cd "$site"

# The deny list is the caller's, and it lives outside the tree being scanned:
# a list sitting in the site root would be found by the scan it armed.
export OKF_SCAN_DENY="$DENY_LIST"

okf-assemble --local "alpha=$FIXTURES/site/alpha" --local "beta=$FIXTURES/site/beta"

# The tenant's own static overlay, which is what a tenant is allowed to add
# and what `okf-check --layouts` does not read the contents of.
cp -r "$PLANTED_STATIC/css/tenant-brand.css" static/css/tenant-brand.css
chmod u+w static/css/tenant-brand.css

# The shipped command, read out of the assembled justfile.
shipped=$(sed -n 's/^[[:space:]]*\(okf-scan .*\)$/\1/p' justfile | head -1)
test -n "$shipped" || fail "the assembled justfile has no okf-scan line to run"
echo "the recipe ships: $shipped"
case "$shipped" in
  *"okf-scan content"*) fail "the build recipe still scans content/ alone" ;;
esac
case "$shipped" in
  *--deny*) ;;
  *) fail "the build recipe's scan is not armed with a deny list" ;;
esac

# 1. The gap, asserted. The old recipe is clean over a planted `static/`.
if ! okf-scan content --deny "$OKF_SCAN_DENY" > "$log/old.log" 2>&1; then
  cat "$log/old.log"
  fail "the content-only scan failed for some other reason; this check no longer measures the gap"
fi
cat "$log/old.log"
grep -q "clean" "$log/old.log" ||
  fail "the content-only scan did not report clean, so step 2 proves nothing"

# 2. The shipped recipe refuses the same tree.
if eval "$shipped" > "$log/planted.log" 2>&1; then
  cat "$log/planted.log"
  fail "the planted tenant name in static/ did not fail the scan"
fi
cat "$log/planted.log"
grep -q "static/css/tenant-brand.css" "$log/planted.log" ||
  fail "the finding was not attributed to the planted stylesheet"
grep -q "denied-term" "$log/planted.log" ||
  fail "the finding was not attributed to the denied-term rule"
grep -q "not scanning public" "$log/planted.log" ||
  fail "the scan did not announce its exclusion; an exemption must never be silent"
if grep -q "$term" "$log/planted.log"; then
  fail "the scanner reproduced the denied term in its own output"
fi

# 3. And it is clean without the plant, so it is a gate rather than noise.
rm static/css/tenant-brand.css
eval "$shipped" > "$log/clean.log" 2>&1 || { cat "$log/clean.log"; fail "the site-root scan is not clean without the plant"; }
cat "$log/clean.log"

# The root scan has to have read more than `content` did, or it would pass
# step 3 by scanning the same files under a different argument.
count() { sed -n 's/.*— \([0-9]*\) file(s) inspected.*/\1/p' "$1"; }
root_files=$(count "$log/clean.log")
content_files=$(count "$log/old.log")
test -n "$root_files" && test -n "$content_files" ||
  fail "could not read the inspected-file counts back out of the scan output"
test "$root_files" -gt "$content_files" ||
  fail "the site-root scan read $root_files files and the content-only scan read $content_files; it is not reaching static/ or layouts/"
echo "site root: $root_files file(s) inspected; content alone: $content_files"

# The shared assets specifically, because they are the ones that publish.
test -f static/css/okf.css || fail "the shared stylesheet is not in the scanned tree"
test -f layouts/_default/baseof.html || fail "the shared layouts are not in the scanned tree"

# The other half: an unarmed build is refused, and refused for saying so.
unset OKF_SCAN_DENY
if okf-assemble --local "alpha=$FIXTURES/site/alpha" --local "beta=$FIXTURES/site/beta" \
  > "$log/unarmed.log" 2>&1; then
  cat "$log/unarmed.log"
  fail "a build whose scan was never armed was allowed to assemble"
fi
cat "$log/unarmed.log"
grep -q "OKF_SCAN_DENY" "$log/unarmed.log" ||
  fail "the refusal did not name the variable an operator has to set"
grep -q "never committed" "$log/unarmed.log" ||
  fail "the refusal did not say the list stays out of every repository"

# An empty list is not an armed scan either: a file with nothing in it is
# indistinguishable from a tenant that never armed the gate.
: > "$log/empty.deny"
export OKF_SCAN_DENY="$log/empty.deny"
if okf-assemble --local "alpha=$FIXTURES/site/alpha" --local "beta=$FIXTURES/site/beta" \
  > "$log/empty.log" 2>&1; then
  cat "$log/empty.log"
  fail "an empty deny list was accepted as an armed scan"
fi
cat "$log/empty.log"
grep -q "holds no terms" "$log/empty.log" || fail "the empty-list refusal did not say why"
