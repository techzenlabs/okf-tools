---
type: "Work Log"
system: "relay"
title: "A stray note"
description: "</ul></li>Status: `Active` | Mode: `Standard` — see [the plan and a stray bracket, count < 5, and a `<div>` written in code"
---

# A stray note

This note and the one beside it share a `system`, so Hugo emits a term page
for it. A term page has no `index.md` to render, which is the one condition
that makes `list.html` fall back to listing the pages it found — and that
listing renders each description through the summary region. It is therefore
the second place a description can close the list holding it, and the first
place nobody thought to check.
