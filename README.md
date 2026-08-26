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
- `okf-serve` serves an assembled site, and refuses to serve it at all when
  the identity header a deployment requires is absent.
- `okf-promote` copies a page from a private bundle into a client-facing one,
  and refuses to write while any link would point somewhere the reader cannot
  follow.

## What conformance means here, and what it does not

OKF §11 is three rules: parseable frontmatter on every non-reserved `.md`, a
non-empty `type` in it, and `index.md` and `log.md` following §8 and §9. That
is the whole of what `okf-check` treats as an error.

The §8 half of that third rule covers one shape that is easy to miss: a page
may not claim the URL its sibling directory's listing publishes at.
`docs/plans.md` beside `docs/plans/index.md` is the case, and [A page and a
section that want one URL](#a-page-and-a-section-that-want-one-url) carries the
measurements.

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

### `stale_after`, and the day it is measured against

`stale_after` is OKF v0.2's staleness slot. Until okf-tools#15 this tool
checked its *shape* and nothing else, so a document could say
`stale_after: 2020-01-01` and no gate anywhere would ever say so. That is the
`keep_readme` defect one layer down, and worse, because the field is in the
document vocabulary rather than in a config file: an author who writes it
reasonably believes the page will be flagged when it lapses.

It is now compared. What it is compared against is a file, not a clock:

```sh
echo 2026-08-25 > .gate-as-of     # repository root, beside okf.toml
```

`okf-check` reports three things, all warnings:

| The document says | The bundle says | The warning |
| --- | --- | --- |
| `stale_after: soon` | anything | is not YYYY-MM-DD |
| `stale_after: 2026-06-14` | no `.gate-as-of` | enforces nothing: this bundle has no `.gate-as-of` naming the day to measure it against |
| `stale_after: 2026-06-14` | `.gate-as-of` holds `2026-06-15` | has passed (as of 2026-06-15) |

Warnings rather than errors because §11 forbids a consumer rejecting a bundle
over a key it does not like, and a review date that lapsed is a quality
matter. `max_warnings` is what gives them teeth: a bundle records the count it
adopted at, so a page going stale spends budget the bundle does not have and
the gate goes red until somebody deals with it.

The named day is the last good day. `stale_after: 2026-06-15` under
`.gate-as-of` of `2026-06-15` is not stale, and `fixtures/staleness/boundary.md`
is there so nobody can quietly change that.

A bundle that sets none of this sees no new output at all. The unenforced
warning fires per document that carries the field, so a bundle with no
`stale_after` anywhere needs no `.gate-as-of` and is told nothing about it.
That is every bundle in the estate today, which is what made shipping this
safe.

#### Why the day is committed rather than read from the clock

`okf-check` runs inside a nix derivation cached on its inputs. A verdict that
reads today's date is computed once and then served from cache: nothing in the
repository changes, the answer changes anyway, and a green default branch
stops being evidence that the check passes today. The red also lands on
whoever next touches the derivation's inputs rather than on whoever let the
date lapse. A sweep of this estate found that shape twenty-one times, four of
which had already bitten somebody (mikeslade/dotfiles#202).

So `.gate-as-of` is a tracked file, which makes it a real derivation input:
the cache invalidates the day it moves and is honest the rest of the time. The
cached green then says something checkable, "as of the day this bundle
committed to, nothing had lapsed", rather than something about a build
machine's calendar. Moving the day is a one-line diff whose gate is red if
something lapsed, which is a reviewable change addressed to whoever owns the
bump.

Nothing in the crate reads a clock. `okf-check --as-of=2026-08-25` measures
against a day you name without committing anything, which is how a person or a
scheduled bump job asks what today would say:

```sh
okf-check --as-of="$(date -u +%F)"
```

A `.gate-as-of` that exists and does not hold a `YYYY-MM-DD` day fails the
run rather than falling back to measuring against nothing. Failing open there
would turn one typo back into the silence this closes.

The field itself stays even though no bundle sets it yet. §11 forbids a
consumer rejecting a bundle over an unknown key, so dropping the validation is
not the same as dropping the field, and `stale_after` is in the v0.2 spec
either way. A vocabulary slot that a consumer must tolerate and this tool
ignores is the worst of the three options.

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

So `okf-tools` owns the eleven layout files, the stylesheet and the `justfile`,
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
copy of `baseof.html` does not. `section-collision` sits beside them too, and
the section below is what it measures.

### A page and a section that want one URL

`docs/plans.md` beside `docs/plans/index.md` cannot be published. The rename
above is what makes `docs/plans/` a Hugo section at `/<id>/plans/`, and
`docs/plans.md` is a page that wants the same URL. Hugo publishes one of them,
exits 0, and says nothing.

Measured on Hugo 0.165, one shape per site, reading the published tree back off
disk rather than reasoning about it:

| source tree | published at `/plans/` | lost |
|---|---|---|
| `plans.md` + `plans/_index.md` + `plans/child.md` | the section | `plans.md` |
| `Plans.md` + `plans/_index.md` | the section | `Plans.md` |
| `plans.md` + `plans/index.md` + `plans/child.md` | the leaf bundle | `plans.md` *and* `plans/child.md` |
| `plans.md` + `plans/child.md`, no listing | the page, and the child still at `/plans/child/` | nothing |
| `plans.md`, no `plans/` | the page | nothing |

Three things follow, and none of them is what reading Hugo's source would have
suggested.

Which file survives is not a property of the shape. On a sixth site whose two
names differ only after sanitisation, `Release Notes (Draft).md` beside
`Release Notes Draft/`, the page won and the section was dropped. The two
bundles that hit this while the tenant sites were being stood up each lost the
other half.

Hugo's silence holds at `--logLevel warn` and under `--printPathWarnings`. That
sanitisation case is the one exception, and it warns, so a build log is not
evidence either way.

The comparison has to be on the published URL segment and not on the name, or
`PLANS.md` beside `plans/` reads as clean. It is not: three repositories in
this estate carry that pair, and a survey that compared names reported all
three clean.

Rows four and five are why a directory carrying no listing is not a section
here. Nothing is lost in either, so a gate that fired on them would be
inventing work for somebody.

`okf-check` reports the collision as an error. `okf-assemble` refuses the tree
before `okf-scan`, hugo and pagefind spend anything on it, and names both
source files and the URL they contend for rather than a byte mismatch found at
the end. The fix is one edit: fold the page into the listing, or rename it to
something that is not its sibling directory's name. `overview.md` is what this
estate calls that file.

`okf-assemble` also refuses a directory holding both `index.md` and
`_index.md`. `std::fs::rename` replaces its destination, so that tree used to
lose the `_index.md` during assembly, silently, which is the same failure the
rename exists to prevent.

**Why an error rather than a warning.** §11 already makes `index.md` following
§8 a conformance rule, and this is a rule about that reserved name: the
`index.md` is what commits the directory to publishing as a section in the
first place. So it lands inside the class §11 gates rather than opening a
fourth one. The ratchet is the wrong instrument besides. `max_warnings` records
what a bundle reported when it adopted, so a bundle adopting with the collision
already present banks it in the budget and never fixes it.

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

The formatted-identifier rule requires the *same* separator twice. A real
identifier is punctuated consistently, and the looser form read a pair of SVG
path coordinates — `649.75 6196` — as one. That fired nine times on a single
generated diagram and refused a 359-document bundle, which is how a
confidentiality gate becomes a file somebody excludes.

A SOPS-encrypted value fails too. The ciphertext is not the secret, but its
algorithm marker says a secrets file reached a corpus that publishes, and the
document around it usually names what the value is for. The marker is not
written out here: this file is inside the tree the self-scan reads, and a
detector its own documentation trips is a detector somebody excludes a file
from. `src/scan.rs` carries it, escaped, where the rule is.

This repository scans itself: `nix build .#checks.x86_64-linux.self-scan`.
### `okf-serve`

```sh
okf-serve --root ./public --bind 100.64.0.1 --port 8080
okf-serve --root ./public --require-header X-MS-CLIENT-PRINCIPAL
```

**Behind a proxy is not a posture. Refusing to serve without proxy-injected
identity is.** A site that merely sits behind Entra's Easy Auth is bypassable
the moment its origin is reachable, and an origin is reachable more often than
anyone plans: a private endpoint misconfigured, a slot swapped, a `$web`
container switched on by somebody adding a storage account. `--require-header`
makes an unauthenticated request fail because *this process* refuses it, so
the access-matrix probe tests a property of the build rather than of a setting.

The three tailnet tenants run the same binary with no flag, because there the
bind address is the boundary and there is no proxy to inject anything. One
binary, two deployments, and the difference is a flag somebody can read in a
unit file.

Three properties beyond serving files, each with a test that was watched
failing:

- **The header check runs before anything touches the filesystem.** An
  unauthenticated request never resolves a path.
- **A directory is never a listing.** It resolves to its `index.html` or to
  404. `okf-assemble` writes an `_index.md` for every section, so a directory
  with no `index.html` is a build that went wrong, and naming its contents to
  a reader is not how to say so.
- **A path that tried to leave the root gets the same answer as a missing
  one.** Containment is asserted twice: `..` textually, and again after
  canonicalising, because a symlink inside the tree pointing out of it does
  not contain the string `..`.

A missing path gets the site's own `/404.html` with a 404 status when the
build produced one. A root that is not a directory is a startup failure, not a
site that answers 404 to everything and looks deployed.

It is not an internet-facing web server, and the header refusal plus those
three properties is the whole of its security argument.

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

An **evidence link** resolves by restating the claim as a dated statement,
with the citation dropped. Keeping the link and marking it unreachable was
considered and rejected: `meetings/2026-07-24-fax-report/summary.md` names a
meeting, its date and its subject, and the raw-markdown publishing route emits
that string whatever a rendered page does with the link.

The statement takes the date and **not** a confidence label. This paragraph
used to say "carrying its own Confirmed / Assumed / Needs-confirmation label",
and that sentence is what produced sixteen manufactured `Confirmed`s on the
one page promotion has drafted: where a citation was dropped, a label appeared
in the hole it left. A restatement carries the label its *source* carries, and
`manufactured-label` below is that rule as a gate rather than as a sentence in
a brief.

What no checker can see is characterisation. A sentence carrying a read on
somebody is forbidden by the same rule that forbids the profile, and no pattern
recognises it. The gate catches pointers, the reviewer catches
characterisation, and the gate is the half that cannot be forgotten.

### The four gates that read the restatement

That paragraph was true of characterisation and over-claimed for four classes
beside it. Each of these is a comparison rather than a judgement, and
`--propose` has both texts in hand. Three reviewers found all four on the one
page promotion has actually drafted.

**A confidence label the source does not carry.** The pilot labelled
twenty-two claims where the source labelled six, and all sixteen additions
said `Confirmed`. The uniform direction is the tell: a habit rather than
sixteen decisions. One of the sixteen was attached to a claim the source never
makes — the evidence had been dropped and a stronger claim took its place,
which is the restatement rule failing in the way that matters most.

The check is a count, not a diff. A restatement rewords the sentence, so the
labels cannot be matched to each other; what can be compared is how many there
are. Dropping a label is fine. Rewording everything is fine. Ending with more
confidence than you started with is not.

**A person-shaped bullet.** Even when the person is at the vendor, and even
when every fact in the bullet is a fact about the system. The pilot's version
paraphrased a named engineer's profile and published his remote access to a
live clinical server. Every name with a profile in the source is known here,
and a bullet is a shape. Restate the system fact without the custodian; if the
person really is the answer to "who owns this", they belong in `owner`.

**A bare register identifier.** `DEC-066`, `OQ-095`. They resolve only inside
the private registers, so they cite evidence the reader cannot reach, and the
numbers disclose how large those registers are. In the pilot, `OQ-095`
resolved to the exact passage the drafter had deliberately cut. An identifier
the destination bundle itself spells out is a reference and passes; a public
standard is exempt by prefix, extended with `[promote]
public_identifier_prefixes`.

**A reconstructed meeting.** "Raised with the vendor at COO level on July 30,
2026 and worked through on a joint call on July 31" names a private
communication, its date and its subject — the disclosure the citation rule
exists to suppress, reassembled in prose. A sentence that carries both an
occasion and a date is refused. A dated claim is required; a dated meeting is
not. And "at COO level" does not de-identify an organisation with one COO.

All four read the body and never the front matter, because `owner` is
list-shaped and full of people. All four skip fenced code, because a quoted log
line is a machine's output rather than a restatement.

### `verified` is dropped, never inherited

The pilot copied `verified: { by: human:mslade, at: 2026-08-21 }` onto a page
that did not exist on 21 August and whose claims were relabelled afterwards.
It asserted that a human had verified text nobody had verified, including the
sixteen manufactured labels. On a bundle whose whole premise is that a client
can trust what it reads, an inherited attestation is worse than none. A
promoted page is unverified until somebody verifies *it*.

### `owner` is the client's side of the table

`owner` answers "who owns this system", and a machine consumer reads it as
exactly that. The pilot's record carried four people, two of them the vendor's,
one with a work address lifted from a signature inside a private escalation
thread — and the split survived only inside free-text `title`, which is not a
split at all.

A destination may name the domains and the organisations it publishes for:

```toml
[[promote.destination]]
name = "knowledge"
path = "../knowledge"
url = "https://docs.example.test/knowledge"
owner_domains = ["example.test"]
owner_orgs = ["Example Works"]
```

An owner whose email is on another domain is refused. An owner with no email —
which is permitted, and which the promotion decision asks to be reported as a
gap rather than guessed at — is placed by the `**Org / role:**` line of the
profile whose title is their name. An owner with neither is a **note**: nothing
here can tell a client custodian from a vendor's account manager, and saying so
is a truer answer than either guess. Both keys are optional and the rule is off
until one is set.


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

The six rules above were each run against the code before them, which is the
only reason to believe them: with the restatement gates removed, the four
restatement tests fail; with `owner_domains` and `owner_orgs` ignored, both
owner tests fail; with `verified` left in the kept keys, the attestation
travels and its test fails.

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
