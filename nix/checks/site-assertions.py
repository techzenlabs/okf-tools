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
        classes = attrs.get("class") or ""
        if tag == "ul" and ("okf-listing" in classes or "okf-cards" in classes):
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


# The front door is one card per bundle and nothing else. It used to be every
# regular page grouped by type, which on a 704-document tenant was 705 list
# items led by 394 work logs. The check that it is gone is worth as much as
# the check that the cards are there: a root that prints the corpus is a root
# that stops working at a size nobody notices crossing.
if "By kind" in home:
    fail("the root still carries the `By kind` dump; it moved into the search filters")

cards = re.search(r'<ul class="okf-cards">(.*?)</ul>', home, re.S)
if not cards:
    fail("the root has no bundle cards")
carded = [href.strip("/").split("/")[0] for href in entries(cards.group(1))]
# Ordered, not just present. `site.Sections` is ordered by `.Title`, and a
# bundle root that carries no front matter has none, so the cards came out
# titleless-first and reordered themselves the day a bundle gained a title.
if carded != ["alpha", "beta"]:
    fail(f"the front door cards {carded}, not both mounted bundles in display-name order")
for bundle in ("alpha", "beta"):
    if f'<a href="/{bundle}/">\n' in home or f'<a href="/{bundle}/"></a>' in home:
        fail(f"the {bundle} card's link has no text in it")

# The filter row and its controls, which are what "By kind" became. They are
# `hidden` in the markup on purpose: the script unhides them once Pagefind has
# filled them, and `hugo server` has no index to fill them from.
for needle in ('id="okf-filters"', 'data-pagefind-key="type"', 'data-pagefind-key="trust"'):
    if needle not in home:
        fail(f"the search filters are missing {needle}")
if not re.search(r'<div id="okf-filters"[^>]*\shidden', home):
    fail("the filter row is not hidden; with no Pagefind index it would render empty selects")

# The breadcrumb. Every crumb is a working link with text in it, the trail
# starts at the bundle and not at the home page the header already links, and
# a section with no `title` — which is every section OKF §8 allows — carries
# its humanised directory name. `2026` is the digit guard: `humanize` turns a
# numeric string into "2026th".
crumbs = Crumbs()
crumbs.feed(read("alpha/archive/2026/awkward-description/index.html"))
# The first crumb is the bundle root's own front-matter title — §8's one
# permitted place — and the two below it are directory names humanised,
# because a non-root `index.md` may not carry a title at all.
expected = [
    ["/alpha/", "Alpha, the first bundle"],
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
# Asserted in both places a description is rendered into a list, because the
# root stopped being one of them and gained the other. The hostile fixture
# description opens `</ul></li>`, which reached the page as raw HTML before
# the parking pass and closed the list holding it, orphaning every entry
# below.
#
#   1. the bundle cards on the front door, where the alpha bundle root
#      carries it — §8 allows front matter exactly there;
#   2. `list.html`'s fallback listing, which renders only where there is no
#      `index.md` to render instead — a taxonomy term page. Two fixture
#      pages share a `system`, so `/system/relay/` lists them both and
#      nothing tested what a description did to that listing until now.
def unbroken(page, where, expected):
    listing = Listing()
    listing.feed(page)
    hostile = expected[0]
    held = [sorted(entries) for entries in listing.lists if hostile in entries]
    if held != [sorted(expected)]:
        fail(f"the {where} holding the awkward description parsed as {held}, not {sorted(expected)}")
    if "&lt;/ul&gt;&lt;/li&gt;Status: <code>Active</code>" not in page:
        fail(f"the awkward description in the {where} did not render as escaped markdown")
    if "`Active`" in page:
        fail(f"a description in the {where} rendered its backticks literally; it needs markdownify")


unbroken(home, "card grid", ["/alpha/", "/beta/"])
unbroken(
    read("system/relay/index.html"),
    "term page's fallback listing",
    ["/alpha/loose/stray-note/", "/alpha/loose/second-note/"],
)

# The counter agrees with itself. `page(s)` is what a listing says when nobody
# looked at it: beta holds one page and alpha holds several.
if "page(s)" in home:
    fail("the bundle cards still hedge with `page(s)`")
counts = re.findall(r'<p class="okf-count">(\d+) page(s?)</p>', home)
if len(counts) != len(carded):
    fail(f"{len(counts)} page counts against {len(carded)} bundle cards")
if any((count == "1") == bool(plural) for count, plural in counts):
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

# And they render on a document only. The front door carries no document
# since the cards replaced the listing, so `TRUST: unverified` there was the
# site telling a reader to distrust the site.
for kind, page in (("front door", home), ("404", read("404.html"))):
    if "okf-facets" in page:
        fail(f"the {kind} carries a trust facet; it makes no claim about a document")

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
kinds = {}
for path in glob.glob(f"{site}/public/pagefind/fragment/*.pf_fragment"):
    body = gzip.open(path).read().decode("utf-8", "replace")
    fragment = json.loads(body.split("pagefind_dcd", 1)[1])
    for key, values in (fragment.get("filters") or {}).items():
        facets.setdefault(key, set()).update(values)
    for tier in (fragment.get("filters") or {}).get("trust", []):
        tiers.setdefault(tier, set()).add(fragment["url"])
    for kind in (fragment.get("filters") or {}).get("type", []):
        kinds.setdefault(kind, set()).add(fragment["url"])

for name in ("type", "status", "team", "trust"):
    if name not in facets:
        fail(f"Pagefind indexed no `{name}` filter")

# Cross-bundle grouping, asserted where it moved to. The root's `By kind` dump
# used to prove that one `type` value gathers pages from every mounted bundle;
# the search filter is what offers that now, so the index is where the
# property has to hold. A filter that fragmented by bundle would give a reader
# asking for decision records one bundle's worth and no sign of the other.
decisions = {url for url in kinds.get("Decision Record", set())}
spread = {url.strip("/").split("/")[0] for url in decisions}
if spread != {"alpha", "beta"}:
    fail(f"the `Decision Record` filter reaches {sorted(spread)}, not both mounted bundles")

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
    f"the front door cards {len(carded)} bundle(s), cross-bundle grouping "
    f"holds in the `type` filter, {len(records)} JSON records match "
    f"{raw_pages} raw pages, and Pagefind carries {sorted(facets)}"
)
