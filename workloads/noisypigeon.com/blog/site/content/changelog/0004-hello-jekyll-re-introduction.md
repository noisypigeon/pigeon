+++
title = "Migrating from Pika to Jekyll"
date = 2026-09-10T07:14:40+00:00
slug = "hello-jekyll-re-introduction"
description = "Short note about migrating my blog from Pika to Jekyll"
aliases = ["posts/hello-jekyll-re-introduction"]
+++


I created [this blog in September 2024](/posts/hihi-pika-lets-consolidate-re-introduction/) with the intention of [consolidating all of my lost writing](/posts/reconnecting-fragments-the-evolution-of-my-names-through-email-accounts/) from over the years into one place. 

While I sometimes contribute new posts, this blog is primarily: my digital shoebox—equal parts time capsule, portfolio, and proof I was here.

Despite loving Pika, I don't use it enough to warrant the USD $60.00/year subscription. So as one does in this day and age, I used Claude to help me create a statically generated equivilent using:

- Jekyll — static site generator.
- kramdown — Markdown → HTML engine.
- Rouge — syntax highlighter for fenced code blocks.
- Liquid — templating language.
- jekyll-feed — generates an Atom RSS feed at /posts_feed.
- GitHub Pages via GitHub Actions.
- CSS — purged/adapted from Pika's original compiled stylesheet to replicate the "gameboy" theme.
- IBM Plex Sans via Bunny Fonts CDN.

If you're curious, you can find the source code [on my GitHub](https://github.com/noisypigeon/noisypigeon.github.io).

♥︎
