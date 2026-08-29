# The planted `static/` overlay

A tenant may add its own files under `static/`, and Hugo copies that tree
byte for byte into `public/`. `css/tenant-brand.css` here carries a synthetic
credential in a comment, which is the shape of the live defect §7.2 recorded:
a shipped asset nothing inspected, published by four tenants with every gate
green.

`nix/checks/scan-site-root.sh` copies this directory into an assembled
fixture tenant and asserts three things in order. That the old
`okf-scan content` recipe passes over it, that the recipe shipped in
`site/justfile` today fails over it, and that the same recipe is clean once
the planted file is gone.

Nothing here is derived from a real corpus. The key is AWS's published
example value and authenticates nothing.
