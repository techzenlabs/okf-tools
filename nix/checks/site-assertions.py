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

# ======================================================================
# The navigation pass: section navigation (A1/A3/A4/A5), the site-wide
# search mount (A2) and sibling navigation at a leaf (A7). Same standard
# as everything above: each assertion was watched failing before it was
# believed, and each failure it guards is a successful build with wrong
# output. The negative controls that produced those failures are recorded
# beside each assertion, and every one was confirmed *applied* — the
# broken shape was seen in `public/` — before the red was trusted,
# because a control that silently fails to apply reports a green that
# reads exactly like a load-bearing gate.
# ======================================================================


def section_body(page, where):
    """The `okf-section` block of a section page, which is where the
    generated listing lives and where its replacement must render."""
    match = re.search(r'<section class="okf-section">(.*?)</section>', page, re.S)
    if not match:
        fail(f"{where} renders no okf-section block")
    return match.group(1)


def nav_of(body, where):
    """The built navigation, and on failure the cause that actually applies.

    Two distinct breaks land here and the message must not conflate them:
    the set-equality proof not firing leaves the generated markers in the
    body, while a proof that fired into the wrong element (a nav demoted to
    `<div>`, which also drops Pagefind's default `<nav>` skip and leaks the
    listing into the index) leaves no markers and no `<nav>` either.
    Watched misattributing: with the element demoted, the old single
    message blamed the proof, which had fired.
    """
    match = re.search(r'<nav class="okf-nav"[^>]*>(.*?)</nav>', body, re.S)
    if not match:
        if "BEGIN OKF INDEX" in body:
            fail(f"{where} still carries the generated block and built no okf-nav; the set-equality proof did not fire")
        fail(f'{where} replaced its generated block but renders no <nav class="okf-nav">; the listing element is missing or demoted')
    return match.group(0)


def h1s(page):
    return [text.strip() for text in re.findall(r"<h1[^>]*>(.*?)</h1>", page, re.S)]


# --- A1, the replaced path. Where the generated block's href set equals
# `.Pages`, the markers are gone, the built nav is there and ignored by
# Pagefind, and its entries are exactly the hrefs the block held — both
# marker forms, because the fixture's legacy `<!-- BEGIN OKF INDEX -->`
# and the current `(tools/okf-index)` form must both match by prefix.
# Watched failing with the template matching the current form exactly:
# every legacy page fell through to verbatim and this reported the marker
# still present on /alpha/runbooks/.
replaced = {
    # legacy marker form
    "alpha/runbooks/index.html": ["/alpha/runbooks/quiet-page/", "/alpha/runbooks/relay-restart/"],
    "alpha/mixed/index.html": [
        "/alpha/mixed/implement/", "/alpha/mixed/plans/",
        "/alpha/mixed/deep/", "/alpha/mixed/guides/", "/alpha/mixed/spare/",
    ],
    "beta/index.html": ["/beta/notes/"],
    # current marker form
    "alpha/bulk/index.html": [f"/alpha/bulk/entry-{i:02d}/" for i in range(1, 22)],
    "alpha/stream/index.html": [f"/alpha/stream/item-{i:02d}/" for i in range(1, 22)],
    "alpha/ledger/index.html": [f"/alpha/ledger/row-{i:02d}/" for i in range(1, 21)],
}
for path, wanted in replaced.items():
    page = read(path)
    if "BEGIN OKF INDEX" in page:
        fail(f"{path} still carries the generated markers; the replacement did not fire")
    body = section_body(page, path)
    nav = nav_of(body, path)
    if "data-pagefind-ignore" not in nav:
        fail(f"{path}'s okf-nav is not data-pagefind-ignore'd")
    if sorted(entries(nav)) != sorted(wanted):
        fail(f"{path}'s built nav lists {sorted(entries(nav))}, not the block's own {sorted(wanted)}")

# The tenant's prose on both sides of the markers survives the swap. The
# mixed fixture writes a lead paragraph above BEGIN and a closing note
# after END. Watched failing with `$head` dropped from the template: the
# lead paragraph vanished and only this noticed.
mixed = read("alpha/mixed/index.html")
body = section_body(mixed, "/alpha/mixed/")
nav = nav_of(body, "/alpha/mixed/")
lead = "This lead paragraph is\ntenant prose outside the markers"
tail = "The closing note, after the generated block"
if lead not in body:
    fail("the mixed section lost its lead paragraph in the replacement")
if tail not in body:
    fail("the mixed section lost the prose after the END marker")
if not body.index(lead) < body.index('okf-nav') < body.index(tail):
    fail("the mixed section's prose did not stay on its own side of the built nav")

# The documents-then-sections partition, in the order okf-index's
# `## Sections` sub-heading promises: documents first, the heading, then
# subsections, each partition in RelPermalink order. The implement/plans
# pair pins the *key*, not merely the direction: `PLANS.md` sorts before
# `implement.md` in filename bytes, and its title `ExecPlan` sorts before
# `Implement`, so byte order, title order and Hugo's weight/date/linkTitle
# default (which collapses to title on this dateless, weightless corpus)
# all list plans first — only RelPermalink ascending lists implement
# first. Watched failing three ways: sort key swapped to "Title", the
# sort removed entirely (Hugo's default), and the direction flipped
# descending; every fixture title elsewhere agrees with its slug, so
# before the ExecPlan retitle the first two of those shipped green.
order = [
    "/alpha/mixed/implement/", "/alpha/mixed/plans/", "okf-nav-heading",
    "/alpha/mixed/deep/", "/alpha/mixed/guides/", "/alpha/mixed/spare/",
]
positions = [nav.index(needle) for needle in order]
if positions != sorted(positions):
    fail(f"the mixed nav is not documents-then-Sections in permalink order: {order} at {positions}")

# --- The fallback. Where the sets differ the block renders verbatim,
# markers and all, and no nav is added. Two shapes:
#
#   1. /alpha/ — the handwritten block omits `untyped.md` and the two
#      `loose/` pages Hugo attaches to the section, so the sets differ by
#      three entries.
#   2. /alpha/journal/ — the divergence is exactly okf-index's `log.md`
#      rule: excluded from every generated listing, published by Hugo
#      like any page. Asserted as an equation, block ∪ {log} = published
#      children, so it is the log page and nothing else that fired the
#      fallback. Watched failing twice: with the fixture's log.md
#      deleted (the sets become equal, the block is replaced, and both
#      the marker and the equation assertions fired), and with the
#      template's set-equality guard removed (every fallback page lost
#      its block).
for path, kept in (
    ("alpha/index.html", ["/alpha/runbooks/", "/alpha/decisions/", "/alpha/documentation/", "/alpha/archive/"]),
    ("alpha/journal/index.html", ["/alpha/journal/entry-one/"]),
):
    page = read(path)
    body = section_body(page, path)
    if "<!-- BEGIN OKF INDEX" not in body or "<!-- END OKF INDEX -->" not in body:
        fail(f"{path} diverges from `.Pages` but lost its generated markers")
    if 'class="okf-nav"' in body:
        fail(f"{path} renders a built nav on the fallback path; divergence must add nothing")
    block = body.split("<!-- BEGIN OKF INDEX")[1].split("<!-- END OKF INDEX -->")[0]
    if sorted(entries(block)) != sorted(kept):
        fail(f"{path}'s generated block holds {sorted(entries(block))}, not all of {sorted(kept)}")

published = {
    "/alpha/journal/" + p.rstrip("/").split("/")[-1] + "/"
    for p in glob.glob(f"{site}/public/alpha/journal/*/")
}
journal_block = section_body(read("alpha/journal/index.html"), "journal")
if set(entries(journal_block.split("<!-- BEGIN OKF INDEX")[1])) | {"/alpha/journal/log/"} != published:
    fail(
        "the journal fallback did not fire on the log.md divergence alone: "
        f"block ∪ log is not the published set {sorted(published)}"
    )

# --- A4. One title per page, and it is the breadcrumb's own string. The
# generated `# <dirname>` H1 is stripped and the template renders the
# `title` region — the same region the crumb renders — so the two cannot
# disagree. Asserted as the one comparison it is: every crumb on the
# deepest fixture page names a section page whose sole H1 renders the
# same text, on the fallback path (/alpha/, where the raw H1 said
# "Alpha") and the replaced path both. Watched failing with the H1 strip
# removed: /alpha/ carried two H1s, the first reading "Alpha" against
# the crumb's "Alpha, the first bundle".
crumbs = Crumbs()
crumbs.feed(read("alpha/archive/2026/awkward-description/index.html"))
if not crumbs.trail:
    fail("the deep fixture page lost its breadcrumb; the H1 agreement below has nothing to compare")
for href, text in crumbs.trail:
    titles = h1s(read(href.lstrip("/") + "index.html"))
    if len(titles) != 1:
        fail(f"{href} renders {len(titles)} H1s; one title per page")
    if titles != [text]:
        fail(f"{href}'s H1 says {titles} while its own breadcrumb crumb says {text!r}")

# --- A5. The section description renders as a lede under the H1, not an
# orphan paragraph above it. Same shape as the facets assertion.
alpha = read("alpha/index.html")
if alpha.index("okf-description") < alpha.index("<h1"):
    fail("the section description renders above the H1; it is a lede, not a preamble")

# A5 at the leaf, where the same orphan read worst: a third of gill's
# described documents (121 of 376, measured) carry a description that is
# a prefix of their own first paragraph, so `single.html` opened with the
# same sentence grey above the title and again below it. The fix is the
# same demotion, not suppression — 255 described documents do not
# duplicate and would lose real summary text. relay-restart carries a
# description *and* its own `# Restart the relay`, so all three parts
# must hold their order: the document's H1, then the lede, then the
# body's first paragraph, with the H1 kept byte-for-byte. Watched
# failing with the description partial left above the H1 in the
# template: the lede-below-H1 comparison fired on this page.
relay = read("alpha/runbooks/relay-restart/index.html")
if len(h1s(relay)) != 1 or "Restart the relay" not in h1s(relay)[0]:
    fail(f"relay-restart's own H1 did not survive the lede split: {h1s(relay)}")
if not relay.index("<h1") < relay.index("okf-description"):
    fail("the document description renders above the H1; it is a lede, not a preamble")
if not relay.index("okf-description") < relay.index("The arrow below is the whole point"):
    fail("the document lede renders below the body; it belongs between the H1 and the first paragraph")

# --- Subsection counts: recursive, and each plural agrees with its own
# number. Same shape as the bundle-card counter above. `Deep` is the
# entry that proves *recursive*: one direct page, one nested a level
# below, so a direct count reads 1 and only `.RegularPagesRecursive`
# reads 2. Watched failing with the template counting `.RegularPages`:
# Deep said "1 page".
counts = re.findall(r'<span class="okf-nav-count">(\d+) page(s?)</span>', nav)
if any((count == "1") == bool(plural) for count, plural in counts):
    fail(f"a subsection counter disagrees with its own plural: {counts}")
expected_counts = {
    "/alpha/mixed/deep/": "2 pages",     # 1 direct + 1 nested: recursion or bust
    "/alpha/mixed/guides/": "2 pages",
    "/alpha/mixed/spare/": "1 page",     # the singular
    "/beta/notes/": "1 page",
}
for href, want in expected_counts.items():
    page = read("beta/index.html") if href.startswith("/beta") else mixed
    entry = re.search(
        re.escape(f'href="{href}"') + r'.*?<span class="okf-nav-count">([^<]*)</span>',
        page, re.S,
    )
    if not entry or entry.group(1) != want:
        fail(f"the {href} entry counts {entry.group(1) if entry else 'nothing'}, not {want!r}")

# --- A3, the fold, on both sides of both lines. 21 documents with prose
# outside the markers folds behind its count; the same 21 with no prose
# stays open, because a page whose listing is its whole content must not
# collapse to one line; 20 or fewer never folds. Watched failing twice:
# with the prose condition forced true (stream folded) and with the
# threshold raised to 99 (bulk stopped folding).
bulk_nav = nav_of(section_body(read("alpha/bulk/index.html"), "/alpha/bulk/"), "/alpha/bulk/")
if '<details class="okf-fold">' not in bulk_nav or "<summary>21 documents</summary>" not in bulk_nav:
    fail("21 documents beside tenant prose did not fold behind their count")
stream_nav = nav_of(section_body(read("alpha/stream/index.html"), "/alpha/stream/"), "/alpha/stream/")
if "<details" in stream_nav:
    fail("a listing that is its page's only content folded; the page is now one line")
# The lower side of the threshold, pinned where a fold can actually fire.
# This assertion used to sit on runbooks, whose two entries have no tenant
# prose outside the markers — so `$prose` blocked its fold at *any*
# threshold and "the threshold is not holding" was a cause that could not
# produce the failure: `$foldAt := 1` shipped fully green through it. The
# mixed section is the fixture that works: its lead paragraph and closing
# note make `$prose` true, so only the threshold decides, and its
# 2-document and 3-section partitions fold the moment the line drops
# below their sizes. Watched failing with `$foldAt := 1`: both partitions
# folded behind their counts and this fired.
mixed_nav = nav_of(section_body(read("alpha/mixed/index.html"), "/alpha/mixed/"), "/alpha/mixed/")
if "<details" in mixed_nav:
    fail("a small listing beside tenant prose folded; the fold threshold dropped below a screenful")
# The line itself, from below. bulk's 21 pins only the top (a threshold
# past 20 stops it folding) and mixed's 2 and 3 pin only the floor, so
# any threshold from 4 to 19 used to ship green — folding the estate's
# 12- and 13-entry listings that sit beside prose. The ledger holds 20
# documents beside tenant prose: `gt` is strict, so at the claimed line
# of 20 it stays open, and any lower line folds it. With bulk's 21 this
# pins the fold to exactly 20. Watched failing on both sides: `$foldAt
# := 19` folded the ledger, `$foldAt := 21` left bulk open.
ledger_nav = nav_of(section_body(read("alpha/ledger/index.html"), "/alpha/ledger/"), "/alpha/ledger/")
if "<details" in ledger_nav:
    fail("20 documents beside tenant prose folded; the fold line dropped below the 20 the template claims")
if len(entries(ledger_nav)) != 20:
    fail(f"the ledger nav holds {len(entries(ledger_nav))} entries, not the 20 that sit on the fold line")
# Every fold summary anywhere agrees with its own plural.
for count, noun, plural in re.findall(r"<summary>(\d+) (document|section)(s?)</summary>", read("alpha/bulk/index.html")):
    if (count == "1") == bool(plural):
        fail(f"a fold summary disagrees with its own plural: {count} {noun}{plural}")

# --- The hostile description inside the *built* nav. The card grid and
# the term page prove the parking pass where the description is rendered
# by the old paths; /alpha/archive/2026/ is now a replaced section whose
# nav renders the same description, so the same proof applies there.
unbroken(
    read("alpha/archive/2026/index.html"),
    "replaced section's built nav",
    ["/alpha/archive/2026/awkward-description/", "/alpha/archive/2026/plain-log/"],
)

# --- The nav's words stay out of the index. Two defences hold this:
# Pagefind skips `<nav>` elements by default (measured: with only the
# attribute removed, nothing leaked), and `data-pagefind-ignore` holds if
# the element ever changes. This assertion guards the *property* — a
# child's description must not become its parent's content — whichever
# defence is on duty. Watched failing with the nav demoted to a `<div>`
# and the attribute dropped: both fragments carried the strings.
ignored_in = {
    "/alpha/archive/2026/": "An ordinary description",   # plain-log's, via the nav
    "/alpha/mixed/": "The lowercase half",               # implement's, via the nav
}
for path in glob.glob(f"{site}/public/pagefind/fragment/*.pf_fragment"):
    body = gzip.open(path).read().decode("utf-8", "replace")
    fragment = json.loads(body.split("pagefind_dcd", 1)[1])
    needle = ignored_in.get(fragment["url"])
    if needle and needle in fragment["content"]:
        fail(f"{fragment['url']} indexes its own nav's child descriptions ({needle!r})")

# ---- A2: one search partial per page, tucked off the root. ------------
# `index.html` mounts the open box on the home page; `baseof.html` mounts
# the collapsed <details> behind `if not .IsHome`. Same predicate,
# opposite polarity, so exactly one fires — and the id count is what
# proves it, because `getElementById` wires only the first `okf-q` and a
# duplicate is a search box that silently does nothing. Counted on every
# generated page including the 404, not just the home page. Watched
# failing with the baseof guard removed: the home page carried two.
pages = sorted(glob.glob(f"{site}/public/**/*.html", recursive=True))
if len(pages) < 30:
    fail(f"only {len(pages)} generated pages; the fixture shrank out from under these assertions")
for path in pages:
    page = open(path, encoding="utf-8").read()
    if page.count('id="okf-q"') != 1:
        fail(f"{path} carries {page.count('id=\"okf-q\"')} search inputs, not exactly 1")
    is_home = path == f"{site}/public/index.html"
    if is_home:
        if "okf-search-tuck" in page:
            fail("the home page renders the tucked search; the root's is the open form")
        if '<section class="okf-search"' not in page:
            fail("the home page lost its open search box")
    else:
        tuck = re.search(r'<details class="okf-search okf-search-tuck"[^>]*>', page)
        if not tuck:
            fail(f"{path} renders no tucked search in its header")
        if re.search(r"\bopen\b", tuck.group(0)):
            fail(f"{path}'s search tuck ships open; collapsed is the point of the tuck")
        header = re.search(r'<header class="okf-header">.*?</header>', page, re.S)
        if not header or "okf-search-tuck" not in header.group(0):
            fail(f"{path}'s search tuck is not inside the header row")
        if not re.search(r'<div id="okf-filters"[^>]*\shidden', page):
            fail(f"{path}'s filter row is not hidden; with no index it renders empty selects")

# The search chrome's words stay out of the index: both forms carry
# `data-pagefind-ignore`, and the fixture prose deliberately never says
# "Kind" or "Clear", so any fragment containing either as a word indexed
# the controls. The tucked form sits inside `<header>`, which Pagefind
# also skips by default; the root's `<section>` has no such cover, and
# that is where the attribute is load-bearing. Watched failing with the
# attribute dropped from the root form: the "/" fragment read
# "Fixture tenant. any any Clear".
for path in glob.glob(f"{site}/public/pagefind/fragment/*.pf_fragment"):
    body = gzip.open(path).read().decode("utf-8", "replace")
    fragment = json.loads(body.split("pagefind_dcd", 1)[1])
    for chrome in ("Kind", "Clear"):
        if re.search(rf"\b{chrome}\b", fragment["content"]):
            fail(f"{fragment['url']} indexes the search chrome ({chrome!r}); the partial lost data-pagefind-ignore")

# ---- A7: sibling navigation at a leaf. --------------------------------
# One estate-wide sweep, because the properties are invariants: at most
# one sibling nav per page, exactly one `aria-current` marker inside it,
# never a self-link, and every href resolves to a page Hugo actually
# published — the `no_index_under` shape is exactly where a naive
# directory listing would emit dead links. Watched failing three ways:
# the current-page span turned back into an anchor (self-link), a
# suffix appended to sibling hrefs (dead link), and the marker span
# removed (no aria-current).
for path in pages:
    page = open(path, encoding="utf-8").read()
    navs = re.findall(r'<nav class="okf-siblings".*?</nav>', page, re.S)
    if len(navs) > 1:
        fail(f"{path} renders {len(navs)} sibling navs")
    if not navs:
        continue
    nav = navs[0]
    own = "/" + path[len(f"{site}/public/") :].removesuffix("index.html")
    if "data-pagefind-ignore" not in nav:
        fail(f"{path}'s sibling nav is not data-pagefind-ignore'd")
    if nav.count('aria-current="page"') != 1:
        fail(f"{path}'s sibling nav marks {nav.count('aria-current=\"page\"')} current pages, not exactly 1")
    hrefs = re.findall(r'href="([^"]*)"', nav)
    if own in hrefs:
        fail(f"{path} links to itself from its own sibling nav")
    for href in hrefs:
        if not glob.os.path.isfile(f"{site}/public{href}index.html"):
            fail(f"{path}'s sibling nav links {href}, which Hugo never published")

# The roll-up at a section-less directory: `loose/` has no index.md, so
# Hugo builds no section for it and its notes attach to /alpha/ — the
# sibling list must therefore reach *outside* the directory (untyped.md
# is alpha's own page), and the "In" heading must link the section that
# actually has a page. Watched failing with the sibling list truncated
# to its first two entries: untyped fell out and only this noticed.
stray = read("alpha/loose/stray-note/index.html")
nav = re.search(r'<nav class="okf-siblings".*?</nav>', stray, re.S)
if not nav:
    fail("the stray note under a section-less directory renders no sibling nav")
nav = nav.group(0)
for sib in ("/alpha/untyped/", "/alpha/loose/second-note/"):
    if f'href="{sib}"' not in nav:
        fail(f"the stray note's siblings omit {sib}; the roll-up to the parent section broke")
if '<h2 class="okf-sib-heading">In <a href="/alpha/">' not in nav:
    fail("the stray note's section heading does not link /alpha/, the nearest section with a page")

def sibling_nav(page, where):
    """The sibling nav, or the authored failure rather than a traceback.

    The assertions below used to chain `.group(0)` onto this search (and
    onto the sibling-list search inside it), so a page that lost its
    sibling nav raised AttributeError instead of the message each
    assertion was written to give. Never a false green — but the message
    is the point, and a traceback names re instead of the nav.
    """
    match = re.search(r'<nav class="okf-siblings".*?</nav>', page, re.S)
    if not match:
        fail(f"{where} renders no sibling nav; a leaf with siblings lost its way out")
    return match.group(0)


# The order is RelPermalink-ascending, not filename bytes, not title,
# and not Hugo's default: `PLANS.md` sorts before `implement.md` in
# bytes, and its title `ExecPlan` sorts before `Implement`, so the pair
# inverts under every candidate key except the permalink — including the
# bare `.RegularPages` default, which on this dateless, weightless
# corpus collapses to title order. Previous/Next walk the same order.
# Watched failing three ways: the sort key flipped descending (plans
# listed first and implement's rel="next" pointed nowhere), the key
# swapped to "Title", and the sort removed entirely.
imp = read("alpha/mixed/implement/index.html")
nav = sibling_nav(imp, "/alpha/mixed/implement/")
siblist = re.search(r'<ul class="okf-listing okf-sib-list">.*?</ul>', nav, re.S)
if not siblist:
    fail("/alpha/mixed/implement/'s sibling nav holds no sibling list; the order below has nothing to check")
siblist = siblist.group(0)
if not siblist.index("okf-sib-here") < siblist.index('href="/alpha/mixed/plans/"'):
    fail("implement does not precede plans in the sibling list; the order is not RelPermalink-ascending")
if 'rel="prev"' in nav:
    fail("the first sibling has a Previous link; the order is not RelPermalink-ascending")
if '<a rel="next" href="/alpha/mixed/plans/">' not in nav:
    fail("implement's Next is not plans; Previous/Next do not walk RelPermalink order")
plans = read("alpha/mixed/plans/index.html")
if '<a rel="prev" href="/alpha/mixed/implement/">' not in plans:
    fail("plans' Previous is not implement; Previous/Next do not walk RelPermalink order")
entry05 = read("alpha/bulk/entry-05/index.html")
if ('rel="prev" href="/alpha/bulk/entry-04/"' not in entry05
        or 'rel="next" href="/alpha/bulk/entry-06/"' not in entry05):
    fail("entry-05's Previous/Next are not entries 04 and 06; the step order broke")

# The sibling fold shares list.html's 20-entry line: 21 siblings fold
# behind "21 documents", 2 stay open. Watched failing with the sibling
# threshold raised: the bulk leaf's list stopped folding.
nav = sibling_nav(entry05, "/alpha/bulk/entry-05/")
if '<details class="okf-fold">' not in nav or "<summary>21 documents</summary>" not in nav:
    fail("21 siblings did not fold behind their count at a leaf")
quiet = read("alpha/runbooks/quiet-page/index.html")
nav = sibling_nav(quiet, "/alpha/runbooks/quiet-page/")
if "<details" in nav:
    fail("a two-sibling list folded at a leaf")
# And the sibling fold's own lower edge, which the two-sibling page pins
# only at the floor: a ledger row has exactly 20 siblings, the strict
# `gt` keeps them open at the claimed line, and any lower line folds
# them. With bulk's 21 this pins the sibling fold to exactly 20 as well.
# Watched failing on both sides: the threshold at 19 folded a ledger
# row's siblings, at 21 the bulk leaf's list stopped folding.
row = read("alpha/ledger/row-07/index.html")
nav = sibling_nav(row, "/alpha/ledger/row-07/")
if "<details" in nav:
    fail("20 siblings folded at a leaf; the sibling fold line dropped below the 20 the template claims")
if nav.count("<li") != 20:
    fail(f"a ledger row's sibling list holds {nav.count('<li')} items, not the 20 that sit on the fold line")

# Fewer than two siblings renders no nav at all: a sibling list of one
# is the page itself. Watched failing with the guard lowered to one.
for path in ("alpha/decisions/0001-pick-a-thing/index.html", "beta/notes/shared-decision/index.html"):
    if "okf-siblings" in read(path):
        fail(f"{path} renders a sibling nav for a page with nothing to step to")

# --- The A1/B2 cancellation, both directions. A generated block whose
# href set equals `.Pages` but which carries a heading okf-index put
# there — month grouping — must fall back, or the months are thrown away
# on a green build with no gate anywhere the wiser. The fixture's
# /alpha/meetings/ links exactly the three pages Hugo knows about under
# two month headings, so set equality alone passes and only the heading
# guard fires.
#
# Two fixtures, not one, because the obvious form of the guard is wrong.
# okf-index writes `## Sections` in every listing that has subdirectories,
# so a guard reading "the block contains an <h2>" would switch the built
# navigation off across the estate. /alpha/mixed/ is that case and it is
# asserted above, in `replaced`, to still take the built navigation; here
# it is asserted that its block really did carry the heading, or the pair
# proves nothing.
meetings = read("alpha/meetings/index.html")
body = section_body(meetings, "/alpha/meetings/")
if "<!-- BEGIN OKF INDEX" not in body or "<!-- END OKF INDEX -->" not in body:
    fail("the month-grouped section lost its generated markers; a heading okf-index wrote is not reproducible and must fall back")
if 'class="okf-nav"' in body:
    fail("the month-grouped section built its own navigation; the month headings are gone from the page")
months = re.findall(r"<h2[^>]*>(.*?)</h2>", body, re.S)
if [m.strip() for m in months] != ["2026-01", "2026-02"]:
    fail(f"the month-grouped block published {months}, not its two month headings")

mixed_src = open(f"{site}/content/alpha/mixed/_index.md", encoding="utf-8").read()
if "## Sections" not in mixed_src:
    fail("the mixed fixture no longer carries a `## Sections` sub-block; the guard's exemption is untested")
mixed_nav = nav_of(section_body(read("alpha/mixed/index.html"), "/alpha/mixed/"), "/alpha/mixed/")
if "Sections" not in mixed_nav:
    fail("the mixed section took the built navigation but rendered no Sections heading")

# --- Lateral movement at a section, symmetric with the leaf. A section
# lists its children and never its peers, so without this a reader at
# /alpha/mixed/deep/ reaches /alpha/mixed/guides/ only by going up.
peers = re.search(r'<nav class="okf-peers"(.*?)</nav>', read("alpha/mixed/deep/index.html"), re.S)
if not peers:
    fail("/alpha/mixed/deep/ renders no peer strip; a section with two or more siblings must offer them")
peers = peers.group(0)
if "data-pagefind-ignore" not in peers:
    fail("the peer strip is not data-pagefind-ignore'd; a section's neighbours are not its content")
if sorted(entries(peers)) != ["/alpha/mixed/guides/", "/alpha/mixed/spare/"]:
    fail(f"the peer strip links {sorted(entries(peers))}, not the two sections beside deep/")
if 'aria-current="page"' not in peers:
    fail("the peer strip links the section it is on instead of marking it")

# Suppressed at a bundle root, where the peers are other bundles and
# moving between bundles is the front door's job. Watched failing with
# the `.IsHome` guard dropped: /alpha/ grew a strip listing /beta/.
if "okf-peers" in read("alpha/index.html"):
    fail("a bundle root renders a peer strip; its siblings are other bundles")

# --- The llms.txt H1. OKF §8 means a section has no title, so `.Title`
# opened every section's reading list with a bare `# ` — the first line
# an agent reads. Asserted over every llms.txt the build published, not
# a sample, because the fix is one line and its blast radius is all of
# them.
bare = [
    path
    for path in glob.glob(f"{site}/public/**/llms.txt", recursive=True)
    if open(path, encoding="utf-8").readline().strip() == "#"
]
if bare:
    fail(f"{len(bare)} llms.txt files open with a bare heading: {sorted(bare)[:3]}")

# --- The acronym table. `humanize` title-cases, which is right for
# `archive` and wrong for `adr`. The table is a lookup on the whole
# directory name and it has to reach every place the title region does:
# the H1, the breadcrumb, the parent's listing entry and llms.txt.
if h1s(read("alpha/adr/index.html")) != ["ADR"]:
    fail(f"the acronym section's H1 is {h1s(read('alpha/adr/index.html'))}, not ADR")
if open(f"{site}/public/alpha/adr/llms.txt", encoding="utf-8").readline().strip() != "# ADR":
    fail("the acronym section's llms.txt heading did not go through the title region")
if ">ADR<" not in read("alpha/adr/0001-record-the-acronym/index.html"):
    fail("the acronym did not reach the leaf page's breadcrumb")

print(
    f"the navigation pass holds: {len(replaced)} replaced sections match "
    "their blocks, 2 divergent sections keep them verbatim, every crumb "
    "agrees with its H1, both folds fire past 20 and stay open at 20, one "
    f"search mount per page over {len(pages)} pages, every sibling href "
    "resolves, the month-grouped block survives verbatim while the "
    "`## Sections` one does not, the peer strip renders below a section "
    "and not at a bundle root, no llms.txt opens with a bare heading, and "
    "`adr` reads ADR"
)
