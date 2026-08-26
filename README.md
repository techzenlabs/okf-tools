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
- `okf-assemble` turns a tenant manifest into a Hugo content tree.
- `okf-scan` refuses to publish a tree carrying a key, a token or an
  identifier.

## What conformance means here, and what it does not

OKF §11 is three rules: parseable frontmatter on every non-reserved `.md`, a
non-empty `type` in it, and `index.md` and `log.md` following §8 and §9. That
is the whole of what `okf-check` treats as an error.

Everything else it reports — vocabulary membership, the `human:`/`process:`
actor form, ISO date shapes, a missing `title` — is a **warning**. A bundle
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

[index]
root_keys = ["okf_version", "title", "description"]
suppress = ["inbox"]                # a drop folder lists nothing
group_by_month = ["meetings"]       # a chronological stream groups by ## YYYY-MM
no_index_under = ["meetings"]       # its children get no index of their own
month_entry_glob = "summary*.md"

[[mirror]]                          # a vendored upstream whose titles carry a tail
paths = ["sources/vendor/docs"]
title_strip = '\s*\|\s*Some Site\s*$'
```

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
not gate — the same conformance-versus-policy line this tool draws, reached
independently, and the strongest signal in the survey that the line sits in the
right place. [`thisismydesign/okf-lint`](https://github.com/thisismydesign/okf-lint)
(MIT) and [`serradura/okf-gem`](https://github.com/serradura/okf-gem)
(Apache-2.0) were also read. None is a dependency: these are single-maintainer
projects against a young spec, and this is load-bearing infrastructure where a
dependency going quiet takes a build with it.

## Licence

MIT or Apache-2.0, at your option.
