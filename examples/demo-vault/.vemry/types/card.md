---
name: card
description: A flashcard — the first line is the question, the body is the answer; the Review view shows it when it is due and writes the grade back onto the line
kind: callout
callout: card
tag: card
icon: ▣
fields:
  - key: due
    type: date
  - key: interval
    type: number
  - key: ease
    type: number
  - key: reviews
    type: number
---
> [!card] {{cursor}}
> 
> [due:: {{date}}]
