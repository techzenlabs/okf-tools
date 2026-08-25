# okf-tools

Conformance checking and index generation for [Open Knowledge Format
v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundles.

Two commands today:

- `okf-check` validates a bundle against OKF §11 and reports the bundle's own
  conventions as warnings.
- `okf-index` regenerates the §8 directory listings from concept frontmatter,
  and with `--check` reports staleness without writing.

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
