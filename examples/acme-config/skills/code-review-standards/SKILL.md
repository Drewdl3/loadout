---
name: code-review-standards
description: Review code against Acme's engineering standards. Use when reviewing a pull request or a diff.
loadout:
  mode: required
  locked: true
  tags: [review, standards]
---

# Acme code review standards

When reviewing a change:

1. Check that every behavior change has a test.
2. Flag any secret, token or credential in the diff as a blocker.
3. Prefer small, focused changes; suggest splitting anything over ~400 lines.
4. Link the ticket (`ACME-1234`) in the summary.
