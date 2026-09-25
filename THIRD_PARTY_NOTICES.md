# Third-party notices

Loadout is licensed under the MIT License (see `LICENSE`).

This file lists third-party code, data tables, or substantial logic that has
been **copied or ported** into this repository, together with the original
copyright and license text, and the files in this repository that derive from
it.

Ordinary dependencies pulled in through Cargo are not listed here; their
licenses ship with their crates.

## Ported material

### runkids/skillshare
Source: https://github.com/runkids/skillshare (commit 4541cb5a36f1e59b721ada75ae06dd690aed3ee0),
`internal/audit/rules.yaml`
Derived files: `audit/default.toml` (a subset of the audit rules, converted to TOML, with
severities remapped and some patterns and messages adapted)

```
MIT License

Copyright (c) 2025 runkids

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

When porting more from [runkids/skillshare](https://github.com/runkids/skillshare)
(MIT), add an entry of the form:

```
### runkids/skillshare
Source: https://github.com/runkids/skillshare (commit <sha>)
Derived files: <paths in this repo>

<full MIT license text with the original copyright line>
```
