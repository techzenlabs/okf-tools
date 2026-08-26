# okf-tools

Conformance checking and index generation for [Open Knowledge Format
v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundles.

Commands:

- `okf-check` validates a bundle against OKF §11 and reports the bundle's own
  conventions as warnings. `--layouts` checks a tenant site repository instead.
- `okf-index` regenerates the §8 directory listings from concept frontmatter,
  and with `--check` reports staleness without writing.
- `okf-migrate` writes the frontmatter it can derive and reports the rest.
  `--retype` applies a rename table to a bundle whose documents are already
  typed, and changes nothing but `type`.
- `okf-assemble` turns a tenant manifest into a Hugo content tree.
- `okf-scan` refuses to publish a tree carrying a key, a token or an
  identifier.
- `okf-promote` copies a page from a private bundle into a client-facing one,
  and refuses to write while any link would point somewhere the reader cannot
  follow.

## What conformance means here, and what it does not

OKF §11 is three rules: parseable frontmatter on every non-reserved `.md`, a
non-empty `type` in it, and `index.md` and `log.md` following §8 and §9. That
is the whole of what `okf-check` treats as an error.

Everything else it reports is a **warning**: vocabulary membership, the
`human:`/`process:` actor form, ISO date shapes, a missing `title`. A bundle
that trips all of them is still conformant, and a foreign consumer is
unaffected. The split matters: §11 forbids a consumer rejecting a bundle for an
unknown `type` or an unknown key, so a tool that errored on those would be
non-conformant itself.

Warnings ratchet rather than gate. `max_warnings` in `okf.toml` records what a
bundle reported when it was adopted; the count may fall and may not rise. The
effect is that best effort means something without ever blocking someone who is
trying to write a document.

## Install

**With nix**, as a flake input, and take the checks with it:

```nix
inputs.okf-tools.url = "github:techzenlabs/okf-tools";
```

```nix
# in a flake-parts repo
imports = [inputs.okf-tools.flakeModules.checks];
```

That adds `checks.okf-conformance` and `checks.okf-index-current`, both reading
`okf.toml`. Per-repo cost is one import and one config file.

**Without nix**, take a release binary. It is built against musl and has no
runtime dependency:

```sh
curl -LO https://github.com/techzenlabs/okf-tools/releases/latest/download/okf-tools-x86_64-unknown-linux-musl.tar.gz
curl -LO https://github.com/techzenlabs/okf-tools/releases/latest/download/SHA256SUMS
sha256sum -c SHA256SUMS
```

`cargo install --git https://github.com/techzenlabs/okf-tools` also works for
anyone who would rather build it.

## `okf.toml`

Sits at the repository root. Every key was a hard-coded constant in the Python
these tools replace; the defaults reproduce those constants, so a bundle with
no `okf.toml` behaves as the originals did.

```toml
config_version = 1          # lets the tool refuse a file it cannot read
okf_version = "0.2"
bundle_root = "."           # "." for a knowledge repo, "docs" for a code repo
title = "…"                 # written into the generated root index.md
description = "…"
max_warnings = 0            # the ratchet

[vocabulary]
extends = ["core", "engineering"]   # presets shipped in the binary
types = ["Model Card"]              # names local to this bundle

[paths]
skip_names = ["node_modules", "__pycache__", "result"]
keep_readme = true          # false retires the name; see below

[index]
root_keys = ["okf_version", "title", "description"]
suppress = ["inbox"]                # a drop folder lists nothing
group_by_month = ["meetings"]       # a chronological stream groups by ## YYYY-MM
no_index_under = ["meetings"]       # its children get no index of their own
month_entry_glob = "summary*.md"

[[mirror]]                          # a vendored upstream whose titles carry a tail
paths = ["sources/vendor/docs"]
title_strip = '\s*\|\s*Some Site\s*$'

[[retype]]                          # okf-migrate --retype, for a typed corpus
from = "Meeting Summary"
to = "Meeting"
```

`[confidentiality]` and `[promote]` are documented under [Promotion, and the
gates that go with it](#promotion-and-the-gates-that-go-with-it). Both default
to inert, so a bundle that names neither behaves exactly as it did before they
existed.

### `keep_readme`, and where README retention actually lives

`keep_readme` records a decision every adopting bundle makes. In a code
repository the name is load-bearing, because a docs gate or GitHub itself
depends on it. In a knowledge bundle §8's generated `index.md` takes the
listing role and the README goes.

`okf-check` warns on every `README.md` still present in a bundle that set the
key to `false`. It deletes nothing: the README to `index.md` move is the one
genuinely destructive step in an adoption, and it stays a reviewed commit
somebody can revert. What makes the warning bite is `max_warnings`, which a
bundle sets to the count it reported when it adopted, so a README that comes
back raises the count and fails the gate.

The key used to record that decision and enforce nothing, which is worse than
either enforcing or deleting it. One bundle set `keep_readme = false` while
deleting every `README.md` by hand, and a reader of that config reasonably
concluded the tool had done the retirement.

### The four vocabularies

Composed per bundle by `extends`, so no bundle ever sees a list it cannot
recall. Two rules generated them, and they decide the next case without another
round of asking:

- **A type says what kind of document this is, never what it is about.**
  Subject is carried by the directory and by the `system`, `project`, `team`
  and `client` keys.
- **A type says what kind of document this is, never what medium carried it.**
  Mail and chat are both `Correspondence`; the medium lives in `tags`.

`core` (5) is in every bundle. `capture` (6) is an engagement's raw record and
never publishes. `knowledge` (6) is enterprise knowledge, and deliberately has
no `Person`. `engineering` (13) is a code repository's `docs/` tree.

A preset is a floor, not a ceiling: a name one repository genuinely needs goes
in that bundle's `types`. A name that shows up in a second repository is a
candidate for a preset, which is a change here and therefore a reviewed one.

### Retyping a corpus that is already typed

`okf-migrate` writes the frontmatter a bundle has not got, and treats a
document that already carries a `type` as finished. That is right everywhere
except one place: a bundle whose 672 documents are typed against 39 names the
vocabulary reduces to 26. `okf-migrate --retype` is that pass.

The rename table is data in `okf.toml`, because the names being retired belong
to the bundle rather than to the tool:

```toml
[[retype]]
from = "Meeting Summary"
to = "Meeting"

[[retype]]
from = "Knowledge Base Page"
review = "Architecture Note, Runbook, Governance Document or Reference, by directory"
```

A row either renames or refers its files to a person, never both and never
neither. The report lists every file a `review` row claims, with the reason,
because those are the only judgements in the pass. It also lists the type names
present in the bundle that no row mentions, so a name somebody forgot to write
a row for looks different from a name that survives on purpose.

Four shapes are refused rather than run, since a table that lies rewrites a
corpus wrongly and nobody reads 672 diffs: a row with neither `to` nor
`review`, a row with both, two rows for one `from`, and a `to` the bundle's
vocabulary does not hold.

**`type` is never removed.** §11 requires the field, so a name being retired
rather than renamed is a `review` row: its files are listed and a person
retypes or deletes each one. A pass that dropped the one required field would
turn a rename into a conformance failure it caused itself.

**It parses frontmatter and never greps.** Two documents in this estate carry a
`^type:` line that is an exemplar inside a fenced code block in a prompt
template, and it is not the document's own type. Over the 757-file reference
bundle `grep -rh '^type:'` finds 68 `Meeting Summary` lines against 67
documents, and the count is how you tell.

Run twice it writes nothing the second time, and it moves nothing but the
`type` value: quoting, key order, spacing and line endings stay the file's own.
Measured over that reference bundle, the whole pass is 493 changed lines across
493 files, and every one of them is a `type:` line.

## The site half

Four tenants each own a site repository holding a manifest and its deploy
configuration. Everything else is here, because four repositories are four
chances to fork a theme and this estate has already paid for that once: two
knowledge repositories carry byte-identical frozen copies of a script whose
upstream fix never reached them. A forked Hugo layout would be worse, since a
layout bug is invisible until somebody reads a page.

So `okf-tools` owns the ten layout files, the stylesheet and the `justfile`,
and `okf-assemble` writes them into the site tree on every build. A tenant
repository tracks its `site.toml`, its deploy configuration and nothing that
`okf-check --layouts` names.

### `site.toml`

```toml
schema_version = 1
tenant = "example"

[site]
title = "Example documentation"
base_url = "https://docs.example.invalid/"

[[bundle]]
id = "handbook"                                  # the stable identity and the mount path
repo = "https://forge.example.invalid/org/handbook.git"
ref = "refs/heads/main"                          # where --update looks for a newer commit
rev = "…"                                        # 40 hex, what this build fetches
subdir = "."                                     # "docs" for a code repository
credential = "example-forge"                     # the name of a secret, never a secret
```

The fetch asks for `rev` and nothing else, so a runner cannot pick up whatever
landed on the branch after the manifest was reviewed. `okf-assemble --update
<id>` resolves `ref`, rewrites `rev` and stops, which makes a roll-forward a
reviewed diff. Every URL is stored as HTTPS even where the local remote is SSH,
because a machine-local `insteadOf` rewrite has no business in a tracked file.

`credentials.allow` beside it lists the credential names this tenant may reach
for. A manifest naming anything else fails on the first line of the job rather
than at fetch time.

### The recipes

`just build` runs assemble, scan, hugo, the raw-markdown comparison and
pagefind. `just serve` adds `hugo server` on localhost and takes `--local
<id>=<path>`, which points one bundle at a working tree for that invocation
only. The override is an argument and is never written to the manifest: when a
working tree and the pinned `rev` disagree, the manifest wins, and the page
footer says `local build`.

`just bundle` writes the assembled tree to `static/bundle.tar.zst`, for an
agent that would rather make one request than several hundred.

### What the site serves a machine

Every page carries its own source bytes at `/<path>/index.md`, and `just build`
compares each pair. The template reads the file rather than `.RawContent`,
which strips frontmatter and would silently drop `type`.

`/index.json` is one flat record per page across every mounted bundle, with
absent values written as `null` rather than omitted so a consumer can index on
shape. `/llms.txt` sits at the tenant root, at every section and on every term
page, so `/team/data-team/llms.txt` is a cross-bundle reading list.
`/build-lock.json` records what the site was built from.

### Six traps, and the fixture behind each

Every one of these produces a *successful build with wrong output*, which is
why Phase 4's exit is fixtures rather than a clean run. Each fixture was
watched failing before it was believed. They build by name:
`nix build .#checks.x86_64-linux.site-pipeline`.

| What goes wrong | What it looks like | Gate |
|---|---|---|
| `index.md` reaches Hugo | Hugo reads the directory as a leaf bundle. Five files render as one page instead of five, and the build reports success | `leaf-bundle-rename` |
| `team = "teams"` in `hugo.toml` | Hugo looks for a `teams:` key, finds none, and emits zero term pages with no warning | `site-pipeline` |
| `{{ .Inner \| htmlEscape }}` in the mermaid hook | `A--&amp;gt;B` renders, whose text content is a literal `A--&gt;B` that mermaid cannot parse as an arrow | `site-pipeline` |
| the mermaid flag read in `<head>` | nothing is emitted, because the body has not rendered yet | `site-pipeline` |
| the script emitted unconditionally | a page with no diagram ships 3.5 MB it never uses | `site-pipeline` |
| an unknown fence language | must render as plain preformatted text rather than failing | `site-pipeline` |

Two more sit beside them. `pinned-commit` builds a source repository with two
commits, pins the first, and asserts that the second's file is absent from the
assembled tree. `layout-fork` asserts that a tenant overlay passes and a tracked
copy of `baseof.html` does not.

### `okf-scan`

It fails closed, and that means three things rather than one. A finding fails.
A file it could not inspect fails, because "unreadable" and "clean" are not the
same answer. And a run that inspected nothing fails, because a scanner pointed
at the wrong directory otherwise reports a clean tree.

Matches are never printed. A finding names the file, the line and the rule, and
shows a masked prefix, because the log a public CI writes is itself a
publication.

The unformatted nine-digit rule is off by default. It fires on any nine
adjacent digits, and in a repository full of commit hashes and pinned revisions
that is mostly noise, so `--bare-9` turns it on where the corpus warrants it.
This repository scans itself: `nix build .#checks.x86_64-linux.self-scan`.
## Promotion, and the gates that go with it

Some bundles are private notes and some are read by a client. `okf-promote`
moves a page across that line, and three rules in `okf-check` keep it from
being crossed by accident.

Promotion **copies**. Nothing moves, nothing is deleted. The source page stays
where it is, keeps every link that made it worth writing, and gains one key:

```yaml
promoted_to: "https://docs.example.test/knowledge/systems/quiet-mill"
```

The promoted page gains the reciprocal, recording the exact commit it came
from:

```yaml
promoted_from:
  repo: "example/notes"
  path: "org/systems/quiet-mill.md"
  rev: "6349f87cb344704da7923ae58b935c03fb0a04d9"
```

References run private to public and never the other way.

### The refusal is the mechanism

```sh
okf-promote --propose org/systems/widget-press.md --to knowledge
```

reads the source, checks the routing table agrees that `knowledge` is where
this page goes, drafts the page to stdout, and prints a **resolution report**
to stderr: every link whose target the destination bundle does not hold, with
the sentence it appears in and what to put there instead. It writes nothing
while that report has an unresolved item, and exits 1. A tool that installed a
page carrying an unresolved pointer into a profile directory would be worse
than no tool, because it would make the disclosure look reviewed.

Two classes of link get different advice, because they need different
resolutions.

A **profile link** resolves into a plain name, or into the page's `owner`
record. A name in prose is contact identity and publishes. A link into a
profile is a pointer at somebody's read on a person, and it does not.

An **evidence link** resolves by restating the claim as a dated statement
carrying its own Confirmed / Assumed / Needs-confirmation label, with the
citation dropped. Keeping the link and marking it unreachable was considered
and rejected: `meetings/2026-07-24-fax-report/summary.md` names a meeting, its
date and its subject, and the raw-markdown publishing route emits that string
whatever a rendered page does with the link.

What no checker can see is characterisation. A sentence carrying a read on
somebody is forbidden by the same rule that forbids the profile, and no pattern
recognises it. The gate catches pointers, the reviewer catches
characterisation, and the gate is the half that cannot be forgotten.

`okf-promote --refresh` answers the question a person cannot answer by looking:
has the source grown a pointer since this page was promoted? It redraws the
draft from the source as it stands, reviews it, and reports the difference
against the draft at the recorded commit. It does **not** install the redraw
over the page. A promoted page is not a mechanical copy, its evidence was
restated by hand, and overwriting it would delete exactly the work the gate
exists to require. The only thing it writes is the new commit, and only when
nothing new is unresolved.

`okf-promote --drift` is the cheap scan: for every page carrying
`promoted_from`, compare the recorded `rev` against the commit that touches
that path now, and list the pages whose source has moved. It runs where both
repositories are checked out, which is one person's machine and never a
client's runner.

### The three rules `okf-check` gains

All three are **off by default**, and that is not timidity. §11 forbids a
consumer rejecting a bundle over an unknown `type` or an unknown key, so a
checker that errored on those unasked would be the non-conformant one. A bundle
opts in because the convention it is buying is a confidentiality boundary
rather than a style preference:

```toml
[confidentiality]
closed_vocabulary = true      # an unknown `type` is an error, not a warning
links_stay_in_bundle = true   # a link the bundle cannot resolve is an error
owner_record = true           # `owner` carries name, title, email, nothing else
site_urls = ["https://docs.example.test/"]   # empty means any http(s) URL passes
```

**No link leaves the bundle.** Containment on its own would not catch the
failure this exists for. A page hand-copied out of a private bundle keeps
`../people/dana-quill.md`, and from `systems/` that resolves to
`people/dana-quill.md`, which is *inside* the new bundle root and simply is not
there. So the target has to exist, and a link to a file the bundle does not
hold is the same error as a link that climbs out of it. `sources` entries are
checked too, resolved against the bundle root, because a frontmatter citation
discloses exactly as a body link does and is not a link.

**`owner` carries exactly `name`, `title` and `email`.** A list, because a
system routinely has both a business owner and a technical one. Any other
subkey is an error. The record is constructed from a source page's owner bullet
cross-checked against a profile, never sliced out of the profile, and the
schema is what keeps it from growing back into one. A prose convention saying
"do not put an assessment here" is a convention. A record with nowhere to grow
is a boundary. `okf-promote` runs the same schema over a hand-restated draft,
because a reviewer building an owner record by hand is exactly when a fourth
subkey appears.

```yaml
owner:
  - name: "Dana Quill"
    title: "Director of Operations"
    email: "dana.quill@example.test"   # optional; the site renders a mailto:
  - name: "Ari Vaughn"
    title: "Platform Lead"
```

**A closed vocabulary where a bundle declares one.** `extends = ["core",
"knowledge"]` gives eleven names and `Person` is not among them, so a
person-shaped page fails the type check outright rather than joining the
warnings somebody meant to get to.

A consumer needs no new flake wiring for any of this: `checks.okf-conformance`
already runs `okf-check`, which already reads `okf.toml`.

### Routing, which is data

A page about a system that has a repository belongs beside that repository's
code, so that a code change and a doc change land in one commit and one review.
No tool can work out which systems those are, so the routing table is written
down, and `--to` naming a different bundle than the route is a refusal rather
than a preference. First match wins, so a page named explicitly beats the shape
that would otherwise sweep it up.

```toml
[promote]
profile_prefixes = ["org/people"]                   # the interpretive layer
evidence_prefixes = ["meetings", "emails", "chats"] # the raw record

[[promote.destination]]
name = "knowledge"
path = "../knowledge"                       # relative to this repository's root
url = "https://docs.example.test/knowledge" # joined to give `promoted_to`

[[promote.route]]
from = "org/systems/quiet-mill.md"
to = "the-mill-repo"
into = "docs/systems"

[[promote.route]]
from = "org/systems/*.md"
to = "knowledge"
into = "systems"

[[promote.source]]      # in the *destination* bundle, for --refresh and --drift
repo = "example/notes"
path = "../notes"
```

`okf-promote` also refuses to install into a destination whose `okf.toml` has
not turned all three confidentiality rules on. Off by default must not come to
mean forgotten.

### Watching the gates fail

`fixtures/confidentiality/` and `fixtures/promotion/` are synthetic bundles
whose pages exist to fail. `cargo test` asserts the exact diagnostic each one
produces, so a rule that stops firing breaks a test rather than quietly
passing a page. Run `okf-check` inside `fixtures/confidentiality` to watch
seven errors, one per way out of the bundle.

## Prior art

`okf-check` and `okf-index` are a port of two Python scripts, not a rewrite,
and the port is proved by parity rather than by inspection: both
implementations were run over a 757-file reference bundle and required to
produce byte-identical indexes and identical diagnostics, including on a run
forced to emit 606 warnings so that ordering and message text were compared at
scale.

Approaches were read from the public ecosystem before any of this was written.
[`cwest/okfctl`](https://github.com/cwest/okfctl) (Apache-2.0) splits
`validate`, which enforces the spec floor, from `lint`, which reports and does
not gate. That is the same conformance-versus-policy line this tool draws,
reached independently, and the strongest signal in the survey that the line
sits in the right place. [`thisismydesign/okf-lint`](https://github.com/thisismydesign/okf-lint)
(MIT) and [`serradura/okf-gem`](https://github.com/serradura/okf-gem)
(Apache-2.0) were also read. None is a dependency: these are single-maintainer
projects against a young spec, and this is load-bearing infrastructure where a
dependency going quiet takes a build with it.

## Licence

MIT or Apache-2.0, at your option.
