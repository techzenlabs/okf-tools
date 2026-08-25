---
type: "Runbook"
title: "Quiet page"
description: "A page with no diagram, which must not pay for the mermaid bundle."
team: "data-team"
status: "draft"
verified:
  - by: "process:fixture-scraper/1.0"
    at: "2026-08-02"
---

# Quiet page

No fences here. The render hook never fires, so the flag it sets is never set,
so this page ships no diagram script. That is the whole assertion.
