# "Proof" (교정) design direction

Static HTML/CSS concepts for the desktop app. Open the HTML files in a browser. The page images (`page-*.svg`) are not committed: they were rendered with rhwp from a third-party public sample, so the page area is empty here.

- `tokens.css`: colour, type, radius and motion tokens, light and dark.
- `concept.css`: component styles (island, rail, page marks, proof slip, panel, capsule).
- `editor.html`, `welcome.html`, `palette.html`: editor, empty state, ⌘K palette.
- `logo.svg`: seal mark (朱文印, seal-script 한). `icon.svg`: 1024 px app icon.

Principles:

1. The page is the interface. Real pages sit on a dotted desk; controls float.
2. One dark island holds the document identity, the state and the commands (⌘K).
3. Edits are proof marks: the person's edits in seal red, agent proposals in blue pencil, numbered in the margin.
4. Affordances replace instructions: hover outlines editable text, hatching marks locked text, buttons carry their shortcuts. The interface has no instruction sentences.
5. Honesty stays visible as short factual labels: the preview engine, "Hancom layout unverified", font substitutions.

Research inputs (9 October 2026): recent.design (Interface feed: agent toolbars with approval and diff chips, step chips with timers, pastel status tags, tactile controls) and designeer.xyz (hairline and dashed rules, monospace counters, key chips, dense dot-separated lists).
