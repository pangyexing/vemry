---
name: requirement
description: One product requirement, with its priority, status and owner on the line
kind: callout
callout: requirement
tag: req
icon: ◆
fields:
  - key: priority
    values: [P0, P1, P2]
    default: P1
  - key: status
    type: state
    values: [draft, reviewing, accepted, shipped, dropped]
    default: draft
  - key: owner
---
> [!requirement] {{cursor}}
> [priority:: {{priority}}] [status:: {{status}}] [owner:: {{owner}}]
