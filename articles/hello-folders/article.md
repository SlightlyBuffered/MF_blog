---
title: The Blog Is Just a Folder
subtitle: A needlessly sensible publishing system built from Markdown, YAML, and spite.
hero: ./assets/hero.svg
publication_date: '2026-09-20T00:00:00+08:00'
release_date: '2026-09-19T08:00:00+08:00'
draft: true
---

## Welcome to the machine

This entire article lives in one directory. The words are in `article.md`; everything else is baggage in `assets/`.

![A tiny folder diagram](assets/folder-diagram.svg)

### It already speaks GitHub-ish Markdown

| Ingredient | Job |
| --- | --- |
| YAML | Front-page metadata |
| Markdown | The actual writing |
| Asset folder | Images without archaeological expeditions |

```python
for article in discover_articles():
    if article.release_at <= now:
        publish(article)
```

> No database was harmed in the making of this blog.
