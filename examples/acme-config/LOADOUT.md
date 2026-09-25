---
loadout: 1
name: acme-config
layer: company
group: acme
owners: ["@acme/platform"]
description: Acme's company-wide agent configuration and the registry of every group.
company:
  name: acme
  layers:
    - { name: company, rank: 0 }
    - { name: org,     rank: 10 }
    - { name: team,    rank: 20 }
    - { name: product, rank: 25 }
    - { name: squad,   rank: 30 }
    - { name: role,    rank: 40 }
  groups:
    # A real company would usually use `{ github_team: "acme/engineering" }`.
    # The example uses environment variables so it runs offline.
    - layer: org
      name: eng
      sources: ["https://git.example.com/acme/eng-skills"]
      membership: { env: { ACME_ORG: eng } }
    - layer: team
      name: payments-dev
      sources: ["https://git.example.com/acme/payments-skills"]
      membership: { any: [ { env: { ACME_TEAM: payments-dev } }, { manual: true } ] }
    - layer: product
      name: billing
      sources: ["https://git.example.com/acme/billing-config"]
      membership: { manual: true }
    - layer: role
      name: developer
      sources: []
      membership: { env: { ACME_ROLE: developer } }
    - layer: role
      name: product-manager
      sources: ["https://git.example.com/acme/pm-presets"]
      membership: { env: { ACME_ROLE: product-manager } }
  policy:
    auto_apply: [company]
    allow_manual_sources: true
  secrets:
    chain: [env, keychain]
---

# Acme company config

Company-wide items live here: every Acme employee gets them, including the
`loadout` skill that teaches every AI tool how to use the Loadout CLI. The `company:`
block above lists every group, the repo(s) it publishes, and how Loadout
decides who belongs to it.
