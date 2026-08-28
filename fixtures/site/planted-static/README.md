# The planted `static/` overlay

A tenant may add its own files under `static/`, and Hugo copies that tree
byte for byte into `public/`. `css/tenant-brand.css` here carries another
tenant's name in a comment, which is the shape of the live defect §7.2
recorded: not a secret, not a document, a name in a shipped asset.

`nix/checks/scan-site-root.sh` copies this directory into an assembled
fixture tenant and asserts three things in order — that the old
`okf-scan content` recipe passes over it, that the recipe shipped in
`site/justfile` today fails over it, and that the same recipe is clean once
the planted file is gone.

Nothing here is derived from a real corpus. `zenith-holdings` is invented.
