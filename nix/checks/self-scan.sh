# A tool that scans for leaks is the first thing scanned.
#
# This repository is public. The rule is no repository inventory, no client
# names, no paths from a client tree, no content and no credentials, and this
# is the gate rather than the habit. The planted fixture is excluded by name,
# and every run prints its exclusions, so an exemption is never silent.
set -euo pipefail
export HOME="$TMPDIR"
tree="$TMPDIR/tree"
cp -r "$SOURCE" "$tree"
chmod -R u+w "$tree"
cd "$tree"
okf-scan --exclude fixtures/scan-planted --exclude fixtures/site/alpha-planted .
