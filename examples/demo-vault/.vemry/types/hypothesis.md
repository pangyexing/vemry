---
name: hypothesis
description: A hypothesis behind a change — the metric, the expected result, and a review date; the review shows up in the Agenda when it is due
kind: task
tag: hypothesis
icon: ⚗
fields:
  - key: metric
  - key: expected
  - key: baseline
  - key: result
  - key: status
    values: [open, validated, refuted, inconclusive]
    default: open
---
{{cursor}} [metric:: {{metric}}] [expected:: {{expected}}] [status:: {{status}}] 📅 {{date}}
