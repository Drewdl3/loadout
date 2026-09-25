---
name: pci-checklist
description: Check a change for PCI-DSS concerns before merging.
loadout:
  applies_to:
    role: [developer]
  tags: [pci, security]
---

# PCI checklist

- No card data in logs, errors or analytics events.
- Card data only through the tokenization service.
- New endpoints handling payments need a security review.
