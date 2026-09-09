---
name: chapter
description: One chapter of a paper being studied — the study skill writes one note per chapter, and the paper's hub lists them with this line's status
kind: paragraph
tag: chapter
icon: ▤
fields:
  - key: status
    type: state
    values: [unread, reading, retold, done]
    default: unread
---
#chapter [status:: {{status}}]
