# This repository's own recipes. Not the tenants' justfile: that one is
# `site/justfile`, and `okf-assemble` writes it into a tenant's site tree.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Every gate this repository has: the package and its tests, rustfmt, Clippy
# and the site checks.
check:
    nix flake check
