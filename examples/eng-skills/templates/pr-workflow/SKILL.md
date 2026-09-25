---
name: pr-workflow
description: How {{team}} works with pull requests. Use when opening, reviewing or merging a PR in {{team}}'s repos.
template:
  description: A pull request workflow skill. Each squad fills in its own conventions and owns the result.
  variables:
    - { name: team, description: "Your squad or team, e.g. Checkout" }
    - { name: reviewers, description: "Approvals needed before merging", default: "2" }
    - { name: branch_prefix, description: "Branch naming prefix", default: "feature/" }
loadout:
  tags: [git, pull-requests]
---

# Pull requests in {{team}}

1. Branch from `main` as `{{branch_prefix}}<ticket>-<short-name>`.
2. Keep PRs small enough to review in one sitting; link the ticket.
3. Fill in the PR template: what changed, why, how it was tested.
4. Approvals required before merging: {{reviewers}} (at least one from {{team}}).
5. Squash-merge once CI is green. Never merge your own PR without review.

See `checklist.md` before asking for review.
