---
name: question
description: An open question in a spec or a research note, with who owns the answer
kind: callout
callout: question
icon: ?
fields:
  - key: owner
  - key: status
    values: [open, answered]
    default: open
---
> [!question] {{cursor}}
> [owner:: {{owner}}] [status:: {{status}}]
