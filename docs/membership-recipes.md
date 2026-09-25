# Membership recipes: identity providers via `exec`

Loadout has built-in providers for GitHub teams, GitLab groups,
environment variables and repo access. For company directories
(Okta, Entra ID, LDAP, …) a company config uses the **`exec`** provider: the
company ships a small command that prints the user's groups, and Loadout
never talks to the identity provider itself or stores its tokens.

## The contract

```yaml
company:
  groups:
    - layer: team
      name: payments-dev
      sources: ["https://git.example.com/acme/payments-skills"]
      membership: { exec: "acme-groups" }
```

- The command runs through `sh -c` (Windows: `cmd /C`) with the user's
  environment and privileges, stdin closed. Commands come from the company config,
  which the company controls.
- It prints JSON on stdout: an array of group names, or `{"groups": [...]}`.
- A group matches if the list contains its `name` (`payments-dev`) or
  `layer:name` (`team:payments-dev`). Emitting `layer:name` avoids clashes
  between groups with the same name in different layers.
- A non-zero exit or invalid JSON is a provider error: Loadout keeps the
  cached profile for that group and warns (it never drops someone from a group
  because the directory was unreachable).
- The same command is run once per sync for all groups that use it; make it
  fast (cache on your side if the directory is slow).

A convention that keeps directory groups and groups in sync: name the
directory groups `loadout-<layer>-<group>` and translate them in the
script, as the recipes below do.

## Okta

Okta's Users API needs a token; most companies don't hand out API tokens to
every employee, so the usual shape is a tiny internal endpoint (behind SSO)
that returns the caller's groups. The script just calls it:

```sh
#!/bin/sh
# acme-groups: print this user's Loadout groups from Okta groups.
# Requires: curl, jq. ACME_GROUPS_URL is your internal endpoint returning
# Okta's /api/v1/users/{id}/groups response for the signed-in user.
curl -fsS --max-time 10 "${ACME_GROUPS_URL:-https://groups.example.com/me}" |
  jq -c '[ .[].profile.name
           | select(startswith("loadout-"))
           | ltrimstr("loadout-")
           | sub("-"; ":") ]'
```

With an API token the user is allowed to hold (e.g. a read-only token in
their keychain):

```sh
#!/bin/sh
token="$(security find-generic-password -s okta-groups -w)"   # macOS keychain
curl -fsS -H "Authorization: SSWS $token" \
  "https://acme.okta.example.com/api/v1/users/${OKTA_USER:?}/groups" |
  jq -c '[ .[].profile.name | select(startswith("loadout-")) | ltrimstr("loadout-") | sub("-"; ":") ]'
```

`loadout-team-payments-dev` becomes `team:payments-dev`.

## Microsoft Entra ID (Azure AD)

Uses the Azure CLI's existing sign-in (`az login`):

```sh
#!/bin/sh
az ad signed-in-user list-member-of --query "[].displayName" -o json |
  jq -c '[ .[] | select(startswith("loadout-")) | ltrimstr("loadout-") | sub("-"; ":") ]'
```

## LDAP / Active Directory

```sh
#!/bin/sh
# Uses the user's Kerberos ticket (-Y GSSAPI); adjust base DN and filter.
ldapsearch -LLL -Q -Y GSSAPI -b "dc=acme,dc=example,dc=com" \
  "(sAMAccountName=$(whoami))" memberOf |
  sed -n 's/^memberOf: CN=loadout-\([^,]*\),.*/\1/p' |
  sed 's/-/:/' | jq -R . | jq -sc .
```

## Windows

`exec` runs through `cmd /C`, so ship a `.cmd`/PowerShell wrapper, e.g.
`membership: { exec: "powershell -NoProfile -File C:\\ProgramData\\Acme\\groups.ps1" }`,
printing the same JSON (`ConvertTo-Json -Compress`).

## Combining providers

Rules combine: `{ any: [ { exec: "acme-groups" }, { manual: true } ] }` lets
people opt in by hand (`lo join team:payments-dev`) when the directory
doesn't know them yet; `{ all: [ { gitlab_group: "acme/payments" }, { exec: "acme-groups" } ] }`
requires both.
