# A gate is believed only after it has been watched failing, and the
# programme has already shipped three that could not fail. Measured on
# 2026-08-27: delete `okf-scan content` from nix/site.nix and every check
# stays green; delete `okf-assemble --verify-raw`, same. Those two steps'
# only effect is to fail, which is exactly why nothing noticed them leaving.
#
# This check closes that. It takes the *actual build script* of
# packages.site — the buildCommand nix/site.nix produced, not a copy of its
# steps — and runs it against three broken fixture tenants, asserting each
# run fails, and fails at the right step for the right reason. Deleting a
# step from site.nix changes the script this check executes, the broken
# build goes green, and this check goes red the same day. Weakening a
# message does the same, because the reason is asserted, not just the exit.
#
# Three scripts, one per gate that must be able to refuse:
#   PLANTED_SCRIPT  a bundle page carries a synthetic private-key block;
#                   `okf-scan content` must stop the build before hugo runs.
#   DRAFT_SCRIPT    a bundle page says `draft: true`; hugo skips it with a
#                   clean exit, and `okf-assemble --verify-raw` must notice
#                   the hole in the rendered surface.
#   DRIFT_SCRIPT    the sources claim a rev that is not the manifest's pin;
#                   `okf-assemble --pinned` must refuse the assembly, which
#                   is what stops a drifted nix/bundles.nix shipping the
#                   wrong corpus under a lock file claiming site.toml's revs.
set -euo pipefail

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# Each script is run the way stdenv runs a buildCommand: -eu, so the first
# failing step stops it. Without -e a red step would be walked past and the
# run could exit green on a later step's success.
run_expecting_failure() {
  local label="$1" script="$2" log="$3"
  local dir="$PWD/$label"
  mkdir -p "$dir"
  if (
    cd "$dir"
    export TMPDIR="$dir"
    export out="$dir/out"
    bash -eu "$script"
  ) > "$log" 2>&1; then
    cat "$log"
    fail "the $label build succeeded; the gate that should refuse it is not in the script"
  fi
  cat "$log"
}

# The breaks are asserted to have landed before any red is believed. The
# private-key header is matched in two halves so this script never carries
# the contiguous marker the scanner looks for.
grep -q -- "-----BEGIN RSA" "$PLANTED_BUNDLE/planted.md" &&
  grep -q "PRIVATE KEY-----" "$PLANTED_BUNDLE/planted.md" ||
  fail "the planted fixture no longer carries the key block"
grep -q "^draft: true$" "$DRAFT_BUNDLE/drafted.md" ||
  fail "the draft fixture no longer says draft: true"
drifted=$(printf 'f%.0s' $(seq 40))
grep -q "$drifted" "$DRIFT_SCRIPT" ||
  fail "the drift script no longer claims the wrong rev"
if grep -q "$drifted" "$TENANT/site.toml"; then
  fail "site.toml pins the very rev the drift script claims; the break did not land"
fi

# 1. The confidentiality gate. The finding must name the file, the line and
#    the rule — a failure for any other reason is a different bug, not this
#    gate firing — and it must fire before hugo, so nothing renders.
run_expecting_failure planted "$PLANTED_SCRIPT" planted.log
grep -q "alpha/planted.md:6" planted.log ||
  fail "the planted key was not attributed to alpha/planted.md line 6"
grep -q "private-key" planted.log ||
  fail "the planted key was not attributed to the private-key rule"
test -d planted/site/content ||
  fail "assembly did not run before the scan refused"
test ! -e planted/site/public ||
  fail "hugo rendered the planted tree; the scan must stop the build first"

# 2. The raw-surface gate. Hugo exits zero over a draft page, so the build
#    must get past hugo and then refuse: the drafted source rendered neither
#    an HTML page nor raw markdown.
run_expecting_failure draft "$DRAFT_SCRIPT" draft.log
grep -q "content/alpha/drafted.md rendered no raw markdown" draft.log ||
  fail "the missing raw markdown was not attributed to the drafted page"
grep -q "content/alpha/drafted.md rendered no HTML page" draft.log ||
  fail "the missing HTML page was not attributed to the drafted page"
test -d draft/site/public ||
  fail "hugo did not run; the draft must fail at --verify-raw, not earlier"

# 3. The pin gate. The sources claim a rev the manifest does not pin, and
#    the refusal must name both commits and the fix.
run_expecting_failure drift "$DRIFT_SCRIPT" drift.log
grep -q "$drifted" drift.log ||
  fail "the refusal did not name the rev the source claimed"
grep -q "$(printf '1%.0s' $(seq 40))" drift.log ||
  fail "the refusal did not name the rev the manifest pins"
grep -q "okf-assemble --bundles" drift.log ||
  fail "the refusal did not name the fix"
test ! -e drift/site/public ||
  fail "a drifted pin rendered a site"
