# The whole pipeline over a synthetic two-bundle tenant, and then every trap
# that produces a *successful build with wrong output*.
#
# Each assertion below was watched failing before it was believed. That is the
# point of the file: none of these six is caught by a non-zero exit anywhere
# else, so a green build proves nothing about any of them.
set -euo pipefail
export HOME="$TMPDIR"
site="$TMPDIR/site"
mkdir -p "$site"
install -m 644 "$FIXTURES/site/tenant/site.toml" "$site/site.toml"
install -m 644 "$FIXTURES/site/tenant/credentials.allow" "$site/credentials.allow"
cd "$site"

# The tenant's brand set, which baseof links by name when the tenant ships it.
# Bytes do not matter here and images are not what is under test: the layout
# asks whether the file is there, and planting four empty ones is what makes
# assertion 8 measure the branch rather than the picture. The negative
# direction — a tenant with no `static/` emitting no icon link at all — is
# packages-site's, whose fixture root has none of these.
mkdir -p static
touch static/favicon.ico static/favicon-32x32.png static/favicon-192x192.png \
  static/apple-touch-icon.png

# The tenant's own stylesheet, same convention. A real one redefines the
# custom properties; an empty one exercises the same branch, because what is
# under test is the link and its position, not the styling.
mkdir -p static/css
touch static/css/tenant-brand.css

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

okf-assemble --local "alpha=$FIXTURES/site/alpha" --local "beta=$FIXTURES/site/beta"
okf-scan . --exclude public
hugo --logLevel warn > hugo.log 2>&1
cat hugo.log
grep -qiE '^(WARN|ERROR)' hugo.log && fail "hugo reported a warning or an error"
okf-assemble --verify-raw
pagefind --site public > pagefind.log 2>&1
cat pagefind.log

# 0. The local-build stamp, positive direction. This pipeline runs `--local`,
#    which is the stamp's real case: a working tree standing in for the pin
#    on somebody's laptop must announce itself on every page. The other
#    direction — a verified `--pinned` build carrying no stamp at all — is
#    packages-site's assertion, so removing the stamp fails there and
#    removing it only for locals fails here: the check can go red both ways.
grep -q 'okf_local_bundles = \["alpha", "beta"\]' hugo.toml ||
  fail "a --local build did not record the overridden bundles in hugo.toml"
grep -q 'local build: alpha, beta' public/index.html ||
  fail "a --local build did not stamp its pages"

diagram="public/alpha/runbooks/relay-restart/index.html"
quiet="public/alpha/runbooks/quiet-page/index.html"

# 1. The taxonomy plural. `team = "teams"` in hugo.toml makes Hugo look for a
#    `teams:` key, find none, and emit no term pages at all — no warning, no
#    error, exit 0. Measured: term pages went from one to none.
test -f public/team/data-team/index.html ||
  fail "no /team/data-team/ term page; the taxonomy value must be the front-matter key, not a plural of it"

# 2. The mermaid arrow. `{{ .Inner | htmlEscape }}` renders A--&amp;gt;B, whose
#    text content is a literal A--&gt;B that mermaid cannot parse as an arrow.
#    Every diagram in the estate uses `-->`.
grep -q 'A\[Relay stops\] --&gt; B' "$diagram" ||
  fail "the mermaid arrow did not survive as A--&gt;B"
if grep -q -- '--&amp;gt;' "$diagram"; then
  fail "the mermaid source was escaped twice; drop htmlEscape from the render hook"
fi

# 3 and 4. The Store flag is read after the content block, and a page with no
#    diagram must not pay for the bundle.
grep -q 'mermaid.min.js' "$diagram" ||
  fail "the diagram page did not load mermaid; the flag is being read before the content renders"
if grep -q 'mermaid.min.js' "$quiet"; then
  fail "a page with no diagram loaded the mermaid bundle"
fi
mermaid_bytes=$(stat -c %s public/js/mermaid.min.js)
echo "mermaid.min.js is ${mermaid_bytes} bytes, and only the diagram page pays for it"

# 5. An unknown fence language degrades quietly rather than failing the build.
grep -q 'class="language-notalanguage"' "$diagram" ||
  fail "the unknown fence language did not render as plain preformatted text"
if grep -q 'notalanguage.*chroma' "$diagram"; then
  fail "an unknown language was handed to the highlighter"
fi
grep -q 'class="chroma"' "$diagram" ||
  fail "the sql fence was not highlighted at build time"

# 6. The 404 page. Measured with the layout removed: Hugo publishes no
#    `public/404.html` at all, and `hugo server` answers a missing path with
#    its own browser-default serif page carrying no stylesheet and no link
#    anywhere. Neither fails a build, so this gate is what notices.
test -f public/404.html || fail "hugo wrote no 404 page"
grep -q 'css/okf.css' public/404.html || fail "the 404 page does not load the stylesheet"
grep -q 'okf-home' public/404.html || fail "the 404 page offers no way back to the site"
if grep -q 'okf-facets' public/404.html; then
  fail "the 404 page carries trust metadata about a document that does not exist"
fi

# 7. GroupBy returns group objects, not a map, and groups across every mounted
#    bundle at once because Hugo's reserved `type` field is OKF's `type`. That
#    one, the breadcrumb, the listing markup and the index hygiene need
#    structured reads rather than greps.
python3 "$ASSERTIONS" "$site"

# 8. The brand set. `favicon.ico` and `apple-touch-icon.png` are fetched from
#    the root by name whatever the markup says, so the two that only a link
#    can reach are the PNGs, and the mark in the header link is markup or it
#    is nothing. Watched failing with the block taken out of baseof: hugo
#    still published all four files, since `static/` is byte-copied whatever
#    the markup says, and not one page referenced any of them.
for rel in 'href="/favicon.ico"' \
  'sizes="32x32" href="/favicon-32x32.png"' \
  'sizes="192x192" href="/favicon-192x192.png"' \
  'rel="apple-touch-icon" href="/apple-touch-icon.png"'; do
  grep -q "$rel" public/index.html || fail "the head links no $rel"
done
grep -q 'class="okf-mark"' public/index.html ||
  fail "the header link carries no brand mark for a tenant that ships one"
# On every page, not only the front door: the mark and the icons live in the
# skeleton, and a reader who lands on a deep page from search sees that page's
# tab first.
grep -q 'class="okf-mark"' public/alpha/runbooks/relay-restart/index.html ||
  fail "a bundle page's header carries no brand mark"
# Decorative, because the title beside it names the link already.
grep -q 'class="okf-mark"[^>]*alt=""' public/index.html ||
  fail "the brand mark is not marked decorative"
# The files themselves reach the published root, or every link above is a 404.
for icon in favicon.ico favicon-32x32.png favicon-192x192.png apple-touch-icon.png; do
  test -f "public/$icon" || fail "the tenant's $icon was not published"
done

# 9. The tenant stylesheet, same convention as the icons and one property
#    more: it must load *after* okf.css, because a tenant restyles by
#    redefining tokens at equal specificity and the cascade breaks ties on
#    source order. A link rendered before okf.css is a build that looks
#    green and a brand that silently loses every tie.
for page in public/index.html public/alpha/runbooks/relay-restart/index.html; do
  grep -q 'href="/css/tenant-brand.css"' "$page" ||
    fail "$page links no tenant-brand.css for a tenant that ships one"
  okf_line=$(grep -n 'href="/css/okf.css"' "$page" | head -1 | cut -d: -f1)
  brand_line=$(grep -n 'href="/css/tenant-brand.css"' "$page" | head -1 | cut -d: -f1)
  test "$okf_line" -lt "$brand_line" ||
    fail "$page loads tenant-brand.css before okf.css, so the tenant loses every cascade tie"
done
test -f public/css/tenant-brand.css ||
  fail "the tenant's stylesheet was not published"
