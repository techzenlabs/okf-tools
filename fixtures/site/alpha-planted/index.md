---
type: "Repository Guide"
title: "Alpha with a planted key"
description: "The negative-control variant of alpha: one page carries a synthetic private-key block, and the site build must refuse to render it."
---

# Alpha, planted

Everything here is synthetic. The page beside this listing exists so the
`site-must-fail` check can watch `okf-scan content` go red inside the
packages.site build script — the same reason `fixtures/scan-planted` exists
for the scanner alone.

<!-- BEGIN OKF INDEX -->
* [Planted](planted/) - The page that must fail the scan.
<!-- END OKF INDEX -->
