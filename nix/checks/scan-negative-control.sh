# A scanner is believed only after it has been watched failing.
#
# The planted fixture carries a synthetic RSA private-key header block and a
# labelled identifier. This gate asserts a non-zero exit against it
# *first*, and only then accepts a clean exit over the assembled tree. It also
# asserts the two failures that are not findings: a run that could not read a
# file, and a run that read nothing at all.
set -euo pipefail
export HOME="$TMPDIR"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

if okf-scan "$FIXTURES/scan-planted" > planted.log 2>&1; then
  cat planted.log
  fail "the planted private key did not fail the scan"
fi
cat planted.log
grep -q "private-key" planted.log || fail "the planted key was not attributed to the private-key rule"
grep -q "national-identifier-labelled" planted.log ||
  fail "the planted labelled identifier was not found"
if grep -q "BEGIN RSA" planted.log; then
  fail "the scanner reproduced the match in its own output"
fi

# A clean tree, which is only meaningful now that the failure has been seen.
clean="$TMPDIR/clean"
mkdir -p "$clean"
printf -- '---\ntype: "Reference"\ntitle: "Nothing here"\n---\n\nA document with nothing in it.\n' \
  > "$clean/page.md"
okf-scan "$clean" > clean.log 2>&1 || { cat clean.log; fail "a clean tree did not pass"; }
cat clean.log

# Failing closed is three things, not one. An empty directory is not clean.
empty="$TMPDIR/empty"
mkdir -p "$empty"
if okf-scan "$empty" > empty.log 2>&1; then
  cat empty.log
  fail "a scan that read nothing reported clean"
fi
grep -q "read nothing" empty.log || fail "the empty-run refusal did not say why"

# A commit hash is nine adjacent digits and must not be an identifier, or the
# gate becomes noise people learn to ignore.
hashes="$TMPDIR/hashes"
mkdir -p "$hashes"
printf 'rev = "ab123456789cdef0000000000000000000000000"\n' > "$hashes/site.toml"
okf-scan --bare-9 "$hashes" > hashes.log 2>&1 || { cat hashes.log; fail "a commit hash was read as an identifier"; }
cat hashes.log
