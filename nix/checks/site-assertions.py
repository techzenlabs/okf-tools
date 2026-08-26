"""The assertions that need to read structured output rather than grep it.

Cross-bundle grouping, the JSON index's shape, the breadcrumb trail, the
listing markup and the Pagefind facets. Each one is a silent failure
elsewhere: a group that fragments, a record that omits a key instead of
nulling it, an anchor with no text in it, a facet that never reached the
index.
"""

import glob
import gzip
import html.parser
import json
import re
import sys


def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


site = sys.argv[1]


def read(path):
    return open(f"{site}/public/{path}", encoding="utf-8").read()


class Crumbs(html.parser.HTMLParser):
    """The breadcrumb trail as (href, text) pairs, empty text included.

    Read with a parser rather than a regex because the failure this guards
    against is an anchor with *no text*, which renders as nothing at all: the
    site shipped a broken breadcrumb for months because a missing link and a
    working one look identical to a reader and to a grep for `okf-crumb`.
    """

    def __init__(self):
        super().__init__()
        self.trail = []
        self._depth = 0
        self._href = None

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "nav" and "okf-crumb" in (attrs.get("class") or ""):
            self._depth = 1
        elif self._depth and tag == "a":
            self._href = attrs.get("href")
            self.trail.append([self._href, ""])

    def handle_endtag(self, tag):
        if tag == "nav" and self._depth:
            self._depth = 0

    def handle_data(self, data):
        if self._depth and self._href is not None and data.strip():
            self.trail[-1][1] += data.strip()


class Listing(html.parser.HTMLParser):
    """Each `okf-listing` list, as the entry links it actually holds.

    A description is markdown rendered into a listing, and a description that
    begins `</ul>` closes the listing that holds it, orphaning every entry
    below. A regex over the same file cannot see that — its own non-greedy
    `</ul>` stops at the injected one, so it reports a short list rather than
    a broken one, and every byte it looks for is still there.
    """

    def __init__(self):
        super().__init__()
        self.lists = []
        self._open = []
        self._item = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "ul" and "okf-listing" in (attrs.get("class") or ""):
            self._open.append([])
        elif tag == "li" and self._open:
            self._item = True
        elif tag == "a" and self._item:
            self._open[-1].append(attrs.get("href"))
            self._item = False

    def handle_endtag(self, tag):
        if tag == "li":
            self._item = False
        elif tag == "ul" and self._open:
            self.lists.append(self._open.pop())

# One `Decision Record` group holding pages from both mounted bundles. Two
# groups, or one holding a single bundle's pages, means grouping fragmented.
home = read("index.html")


def entries(body):
    """The first href in each `<li>`, which is the entry's own link.

    Not every href in the group: a description is rendered markdown now, so a
    description carrying a link would otherwise count as an entry.
    """
    found = []
    for item in body.split("<li")[1:]:
        match = re.search(r'href="([^"]+)"', item)
        if match:
            found.append(match.group(1))
    return found


groups = {
    m.group(1): entries(m.group(2))
    for m in re.finditer(r"<h3>(.*?)</h3>\s*<ul[^>]*>(.*?)</ul>", home, re.S)
}
decisions = groups.get("Decision Record", [])
bundles = {href.strip("/").split("/")[0] for href in decisions}
if bundles != {"alpha", "beta"}:
    fail(f"`Decision Record` grouped {bundles}, not both mounted bundles")

# The breadcrumb. Every crumb is a working link with text in it, the trail
# starts at the bundle and not at the home page the header already links, and
# a section with no `title` — which is every section OKF §8 allows — carries
# its humanised directory name. `2026` is the digit guard: `humanize` turns a
# numeric string into "2026th".
crumbs = Crumbs()
crumbs.feed(read("alpha/archive/2026/awkward-description/index.html"))
expected = [
    ["/alpha/", "Alpha"],
    ["/alpha/archive/", "Archive"],
    ["/alpha/archive/2026/", "2026"],
]
if crumbs.trail != expected:
    fail(f"the breadcrumb read {crumbs.trail}, not {expected}")

for path in glob.glob(f"{site}/public/**/index.html", recursive=True):
    trail = Crumbs()
    trail.feed(open(path, encoding="utf-8").read())
    for href, text in trail.trail:
        if not text:
            fail(f"{path} renders a crumb to {href} with no text inside the anchor")
        if href == "/":
            fail(f"{path} repeats the home page in its breadcrumb")

top = Crumbs()
top.feed(read("alpha/index.html"))
if top.trail:
    fail(f"a top-level section carries a breadcrumb ({top.trail}); its only ancestor is home")

# The tab title. The home page's own title *is* the site title, and a section
# named for a year is not the 2026th of anything.
if "<title>Fixture tenant</title>" not in home:
    fail("the home page repeats the site title in its own <title>")
if "<title>2026 &middot; Fixture tenant</title>" not in read("alpha/archive/2026/index.html"):
    fail("the year directory's tab title is not 2026; check the digit guard in the title fallback")

# A description is markdown, and only markdown. The awkward fixture opens with
# `</ul></li>`, which reached the page as raw HTML before the parking pass and
# closed the listing that held it.
listing = Listing()
listing.feed(home)
awkward = "/alpha/archive/2026/awkward-description/"
plain = "/alpha/archive/2026/plain-log/"
held = [entries for entries in listing.lists if awkward in entries]
if held != [[awkward, plain]]:
    fail(f"the listing holding the awkward description parsed as {held}, not one list of two")
if "&lt;/ul&gt;&lt;/li&gt;Status: <code>Active</code>" not in home:
    fail("the awkward description did not render as markdown with its raw HTML escaped")
if "`Active`" in home:
    fail("a description rendered its backticks literally; it is markdown and needs markdownify")

# The counter agrees with itself. `page(s)` is what a listing says when nobody
# looked at it: beta holds one page and alpha holds several.
if "page(s)" in home:
    fail("the bundle list still hedges with `page(s)`")
counts = re.findall(r"&mdash; (\d+) page(s?)\n", home)
if not counts or any((count == "1") == bool(plural) for count, plural in counts):
    fail(f"the page counter disagrees with its own plural: {counts}")

# An empty taxonomy is rendered, and rendered politely. §11 forbids refusing
# to render it; an H1 over a zero-item list is not rendering it either.
if "Nothing is filed under" not in read("client/index.html"):
    fail("an empty taxonomy renders an empty list instead of a sentence")

# The facets are reference values a reader consults after deciding the page is
# relevant, so they render below the content and not above the H1.
page = read("alpha/runbooks/relay-restart/index.html")
if page.index("okf-facets") < page.index("<h1"):
    fail("the facets render above the title; move the partial below the main block")

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

# The facets are inside `data-pagefind-body` and carry `data-pagefind-ignore`,
# which is what keeps the filters above while the words leave the index.
# Measured before the attribute: every page's indexed text began "trust
# unverified", so a search for "unverified" matched the whole estate.
for path in glob.glob(f"{site}/public/pagefind/fragment/*.pf_fragment"):
    body = gzip.open(path).read().decode("utf-8", "replace")
    fragment = json.loads(body.split("pagefind_dcd", 1)[1])
    for tier in ("unverified", "machine-confirmed", "human-reviewed"):
        if f"trust {tier}" in fragment["content"]:
            fail(f"{fragment['url']} indexes the facet block; the `<dl>` needs data-pagefind-ignore")
    if fragment["url"] == "/" and "cross-bundle grouping proof" in fragment["content"]:
        fail("the home page indexes its own copy of the corpus; ignore the root listings")
    if fragment["url"].endswith("404.html"):
        fail("the 404 page is a search result; it needs data-pagefind-ignore")

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
