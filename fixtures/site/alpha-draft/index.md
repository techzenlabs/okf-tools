---
type: "Repository Guide"
title: "Alpha with a draft page"
description: "The negative-control variant of alpha: one page says draft: true, Hugo declines to render it, and --verify-raw must notice the hole."
---

# Alpha, drafted

The page beside this listing carries `draft: true`. Hugo skips it with a
successful exit, so the only gate that can notice is `okf-assemble
--verify-raw`, which walks `content/` and finds a source with neither an HTML
page nor raw markdown beside it. The `site-must-fail` check watches exactly
that refusal.

<!-- BEGIN OKF INDEX -->
* [Rendered](rendered/) - A page that renders, so the build gets as far as hugo.
* [Drafted](drafted/) - The page Hugo declines to render.
<!-- END OKF INDEX -->
