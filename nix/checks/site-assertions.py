"""The assertions that need to read structured output rather than grep it.

Cross-bundle grouping, the JSON index's shape, and the Pagefind facets. Each
one is a silent failure elsewhere: a group that fragments, a record that omits
a key instead of nulling it, a facet that never reached the index.
"""

import glob
import gzip
import json
import re
import sys


def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


site = sys.argv[1]

# One `Decision Record` group holding pages from both mounted bundles. Two
# groups, or one holding a single bundle's pages, means grouping fragmented.
home = open(f"{site}/public/index.html", encoding="utf-8").read()
groups = {
    m.group(1): re.findall(r'href="([^"]+)"', m.group(2))
    for m in re.finditer(r"<h3>(.*?)</h3>\s*<ul>(.*?)</ul>", home, re.S)
}
decisions = groups.get("Decision Record", [])
bundles = {href.strip("/").split("/")[0] for href in decisions}
if bundles != {"alpha", "beta"}:
    fail(f"`Decision Record` grouped {bundles}, not both mounted bundles")

# The JSON index: one record per page with a source, absent values as null
# rather than omitted, so a consumer can index on shape.
records = json.load(open(f"{site}/public/index.json", encoding="utf-8"))
required = {
    "path", "raw", "type", "title", "team", "project",
    "bundle", "trust", "verified", "status", "stale_after", "sources",
}
for record in records:
    missing = required - set(record)
    if missing:
        fail(f"{record.get('path')} omits {sorted(missing)} instead of nulling them")

untyped = [r for r in records if r["path"] == "/alpha/untyped/"]
if not untyped or untyped[0]["type"] is not None:
    fail("an untyped page must appear with a null type, never be rejected (§11)")

raw_pages = len(glob.glob(f"{site}/public/**/index.md", recursive=True))
if len(records) != raw_pages:
    fail(f"{len(records)} JSON records against {raw_pages} raw markdown pages")

# The §5.3 trust tiers, derived and never stored, and the three Pagefind
# facets beside them.
facets = {}
tiers = {}
for path in glob.glob(f"{site}/public/pagefind/fragment/*.pf_fragment"):
    body = gzip.open(path).read().decode("utf-8", "replace")
    fragment = json.loads(body.split("pagefind_dcd", 1)[1])
    for key, values in (fragment.get("filters") or {}).items():
        facets.setdefault(key, set()).update(values)
    for tier in (fragment.get("filters") or {}).get("trust", []):
        tiers.setdefault(tier, set()).add(fragment["url"])

for name in ("type", "status", "team", "trust"):
    if name not in facets:
        fail(f"Pagefind indexed no `{name}` filter")

expected = {
    "human-reviewed": "/alpha/runbooks/relay-restart/",
    "machine-confirmed": "/alpha/runbooks/quiet-page/",
    "unverified": "/alpha/untyped/",
}
for tier, url in expected.items():
    if url not in tiers.get(tier, set()):
        fail(f"{url} is not filed under trust `{tier}`")

print(
    f"cross-bundle grouping holds, {len(records)} JSON records match "
    f"{raw_pages} raw pages, and Pagefind carries {sorted(facets)}"
)
