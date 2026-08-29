# The scan reads what the site publishes, not just what it was written from.
#
# For as long as the build recipe read `okf-scan content`, `static/` and
# `layouts/` were outside every gate in the estate — and those are the two
# trees the shared assets land in. Hugo byte-copies `static/` into `public/`,
# so a comment in a stylesheet is served to every reader of every tenant.
# That defect was live, on a real file, and nothing here would have caught it.
#
# So this check asserts three things in order, against a fixture that plants a
# synthetic credential in `static/`:
#
#   1. `okf-scan content` — the recipe as it stood — passes over the planted
#      tree. That is the gap, and it is asserted rather than described.
#   2. The command `site/justfile` ships today fails over the same tree,
#      names the planted file and the rule, and does not reproduce the match.
#   3. The same command is clean once the planted file is gone, so it is a
#      gate and not a tripwire that fires on everything.
#
# The command in step 2 is read out of the *assembled* justfile rather than
# written here, so weakening the recipe changes what this check runs and the
# check goes red the same day. That is site-must-fail's principle applied to
# a recipe line instead of a build script.
set -euo pipefail
export HOME="$TMPDIR"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# The planted value is read out of the fixture, never written here. This
# repository scans itself, so a check that spelled the credential out would
# be a finding in its own gate.
planted="$PLANTED_STATIC/css/tenant-brand.css"
key=$(grep -o 'AKIA[0-9A-Z]\{16\}' "$planted" | head -1)
test -n "$key" ||
  fail "the planted stylesheet no longer carries a credential; the break did not land"

# Logs live outside the tree being scanned. A log inside it is a file the
# next scan reads, which moves the inspected-file count this check compares.
log="$TMPDIR/logs"
mkdir -p "$log"

site="$TMPDIR/site"
mkdir -p "$site"
install -m 644 "$FIXTURES/site/tenant/site.toml" "$site/site.toml"
install -m 644 "$FIXTURES/site/tenant/credentials.allow" "$site/credentials.allow"
cd "$site"

okf-assemble --local "alpha=$FIXTURES/site/alpha" --local "beta=$FIXTURES/site/beta"

# The tenant's own static overlay, which is what a tenant is allowed to add
# and what `okf-check --layouts` does not read the contents of.
cp -r "$planted" static/css/tenant-brand.css
chmod u+w static/css/tenant-brand.css

# The shipped command, read out of the assembled justfile.
shipped=$(sed -n 's/^[[:space:]]*\(okf-scan .*\)$/\1/p' justfile | head -1)
test -n "$shipped" || fail "the assembled justfile has no okf-scan line to run"
echo "the recipe ships: $shipped"
case "$shipped" in
  *"okf-scan content"*) fail "the build recipe still scans content/ alone" ;;
esac

# 1. The gap, asserted. The old recipe is clean over a planted `static/`.
if ! okf-scan content > "$log/old.log" 2>&1; then
  cat "$log/old.log"
  fail "the content-only scan failed for some other reason; this check no longer measures the gap"
fi
cat "$log/old.log"
grep -q "clean" "$log/old.log" ||
  fail "the content-only scan did not report clean, so step 2 proves nothing"

# 2. The shipped recipe refuses the same tree.
if eval "$shipped" > "$log/planted.log" 2>&1; then
  cat "$log/planted.log"
  fail "the planted credential in static/ did not fail the scan"
fi
cat "$log/planted.log"
grep -q "static/css/tenant-brand.css" "$log/planted.log" ||
  fail "the finding was not attributed to the planted stylesheet"
grep -q "aws-access-key-id" "$log/planted.log" ||
  fail "the finding was not attributed to the aws-access-key-id rule"
grep -q "not scanning public" "$log/planted.log" ||
  fail "the scan did not announce its exclusion; an exemption must never be silent"
if grep -q "$key" "$log/planted.log"; then
  fail "the scanner reproduced the planted credential in its own output"
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
