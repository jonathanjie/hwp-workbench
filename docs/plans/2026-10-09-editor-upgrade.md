# Editor upgrade plan — real files, agent skill, modern desktop

Date: 2026-10-09 · Owner: orchestrating Claude session · Implementers: GPT 6.1 Sol workers (`router-sol`)

## Goal

Make HWP Workbench the most usable and agent-friendly HWP/HWPX editor available, without weakening the preservation contract in `AGENTS.md`:

1. Open real HWP/HWPX files (the public corpus failed before this plan).
2. Show real pages, not a structure approximation, using `@rhwp/core` 0.8.7 as a render-only adapter.
3. Let Claude Code and Codex edit documents through an installable skill, the CLI and MCP, with a live link to the open desktop app.
4. Give the desktop a distinctive, modern interface (design direction in section 5).
5. Later: explicit export between HWP, HWPX and DOC/DOCX with a loss report.

The Python engine stays the authoritative save path. rhwp renders and independently verifies; it does not replace native operations.

## Evidence at plan start

| Check | Result |
|---|---|
| `python3 scripts/run-tests.py` (macOS 26 arm64, Python 3.14.6) | 68 passed |
| Real Electron 44.5.1 launch (Playwright `_electron`) | Window opened, demo HWPX opened, no console errors |
| Public corpus, `pyhwp-aligns.hwp`, `pyhwp-borderfill.hwp` | **Failed to open**: compressed streams end with an 8-byte trailer (CRC32 + uncompressed size, little-endian) that the engine rejects |
| Public corpus, `hwp-cli-report-tables.hwpx` | Parsed, no-op byte identical, edit reopened |
| Both public HWP files | Contain Hancom's default `Scripts/DefaultJScript` stub, so the engine would mark them read-only even after the trailer fix |
| rhwp 0.8.7 on public files | Renders all three (16, 1 and 6 pages); HWPX→HWP export reports 0 losses |
| rhwp 0.8.7 on our synthetic fixtures | **Rejects both**: HWPX manifest href lacks `Contents/`; HWP reports a corrupt deflate stream. The fixtures are not realistic files |

## Work breakdown

Each task runs in its own git worktree and branch under `../hwp-workbench-worktrees/`. One writer per file set. Workers commit as they go and write `work/results/<task>.json` in their worktree (fields: `task`, `done`, `commits`, `testsRun`, `testsPassed`, `skipped`, `notes`, `openIssues`). The orchestrator merges into `main`, runs all checks and does visual QA.

### Wave 1 (parallel)

**A1 — Real-file opening** (`feat/engine-real-files`; owns `engine/hwp_workbench/document.py` HWP paths, `cfb.py`, `tests/`, `fixtures/`, `scripts/generate-fixtures.py`, `scripts/test-public-corpus.py`)
- Accept an optional 8-byte gzip-style trailer after each raw-deflate HWP stream only when CRC32 and size match; reject any other trailing data. On rewrite, regenerate the trailer for streams that had one. Regression tests with synthetic streams and the public corpus.
- Classify Hancom's default script stub as inert by exact match of the decompressed `Scripts/DefaultJScript` and `Scripts/JScriptVersion` payloads (record the known variant hashes in code with a comment). Keep the streams byte-identical, never execute them, add an `info` warning `DEFAULT_SCRIPT_STUB`. Any other script content still blocks editing.
- Make both synthetic fixtures open in rhwp 0.8.7 (realistic `content.hpf` hrefs, valid compressed `DocInfo`, and whatever else rhwp requires). Keep them self-authored.
- Exit: public HWP files open; edit, save and reopen pass; all tests pass.

**B1 — Agent operations** (`feat/agent-api`; owns `service.py`, `mcp.py`, `cli.py`, `errors.py`, new `engine/hwp_workbench/agent_ops.py`, `tests/test_agent_ops.py`, `docs/AGENT-INTERFACE.md`)
- New shared operations for CLI, MCP and desktop RPC: `read` (compact, token-efficient Markdown-style view with node IDs; tables as grids; `offset`/`limit` paging), `search` (literal or regex, case options, returns node IDs, offsets and context), `replace` (find/replace that returns a ready patch plus diff; applies only with `output` and acknowledgement), `fill` (fill form cells by label: normalised label match, value goes to the next cell in the row, else the cell below; reports unresolved labels), `set` (one node, one text → patch).
- `inspect --compact` that omits `view` and `parts`.
- Every error carries `hint` (the recovery step) and `retryable`.
- Exit: CLI and MCP parity tests for each operation; docs updated.

### Wave 2

**A2 — Independent verification and anchors** (after A1 merge)
- `engine/bridge/rhwp-bridge.mjs`: Node helper that reads one JSON request plus document bytes on stdin (never a path), with operations `probe` (page count, section/paragraph texts), `render` (SVG for one page) and `export` (HWP or HWPX bytes plus rhwp's `contentLoss` report). Python wrapper `independent.py`: bounded subprocess, timeout, returns `{available: false, reason}` when Node or the package is missing.
- `apply` adds `independentCheck` (rhwp reopens the output and finds each edited text). Tests skip with a stated reason when the bridge is unavailable; the reports list them as skipped.
- Node `anchor`: `{section, paragraph, charOffset, inCell}` using rhwp-compatible top-level paragraph indexes, cross-checked against rhwp `getTextRange` on the public files.

**B2 — Agent link and skill** (after B1 merge)
- `propose`: writes a proposal (patch, title, author, createdAt) to `<document folder>/.hwp-workbench/proposals/` with no-overwrite workspace writes. `session`: reads `<document folder>/.hwp-workbench/session.json`, which the desktop writes only while the user enables Agent link.
- `render`: page SVG through the bridge, and PNG on macOS through `qlmanage` when requested.
- Skill `skills/hwp-workbench/SKILL.md` with references (patch format, recipes: fill a form, find and replace, hand a proposal to the user). `scripts/install-agent-skill.sh` links it into `~/.claude/skills/` and `~/.codex/skills/` and refuses to replace anything that is not its own link. Example `.mcp.json`.

**D — Variable-length HWP text** (after A2)
- Length-changing replacement inside a paragraph: update `PARA_HEADER` character count, `PARA_CHAR_SHAPE` positions, `PARA_LINE_SEG` start positions, `PARA_RANGE_TAG` ranges; refuse edits that cross controls. Verify with rhwp reopen and render. Keep the warning that Hancom layout is unverified.

**C — Desktop** (design-led; starts after the design direction in section 5 is approved)
- C1 shell, design system, rhwp page view, click-to-edit popover, Electron end-to-end test.
- C2 command palette, find and replace, changes panel, review sheet, toasts, checks panel, shortcuts, i18n and accessibility.
- C3 (after A2 and B2) live preview of staged edits, Agent link panel with proposal inbox and session sharing, and an Agents card that shows and installs the skill and MCP setup for Claude Code, Codex and Claude Desktop.

- C4 (after C3) Windows 한글 familiarity on the Mac: a "Windows 한글" keymap (Ctrl → ⌃ and ⌘, Alt → ⌥, chords), 한글 vocabulary in the Korean UI and palette, read-only 글자 모양/문단 모양/편집 용지 cards, 문자표, Tab between table cells, a 조판 부호 overlay and a missing-font list with Mac substitutes. Key table for the user to confirm: `docs/design/hangul-keymap.md`.

Decision (user, 9 October): Save in the desktop app replaces the opened file, as in 한글. It must verify the new file first, replace atomically, keep the previous version (newest 10 per document) and refuse when the file changed on disk. Save As writes a new file. Agents keep writing new files or proposals by default. Engine part: brief S (`save` operation); desktop part: C4 task 0.

**P — Apple Silicon app** (after C4 and F): `HWP Workbench.app` and a DMG for arm64 with a bundled Python runtime (no Python or Node needed on the target Mac), rhwp included, Electron running the rhwp bridge as Node, `.hwp`/`.hwpx` document types, the seal icon, a CLI inside the app for Claude Code and Codex, ad-hoc signing and a short Korean install note. Notarization needs an Apple Developer account; that is the user's decision.

Decision (user, 10 October): best effort for the first user; they adjust details with their own Claude Code or Codex, so key tables and settings stay in one plain, documented place. Their Mac has an Apple chip.

Decision (user, 9 October): the first end user works daily in Windows 한글 and will use HWP Workbench on a Mac. Familiar keys and words come first; the visual design stays modern.

Decision (user, 9 October): no chat panel inside the editor. People keep working in their own Claude Code, Codex or Claude app; the editor is the live visual and review surface for what those agents propose.

**B3 — Reach agents where people already work** (after B2)
- Claude Desktop: an MCP Bundle (`.mcpb`, stdio) for one-click install, with the workspace folder chosen at install time.
- Claude Code and Codex: documented one-line MCP setup (`claude mcp add …`, `codex mcp add …`) next to the skill.
- `hwp-workbench open <path>`: brings the document up in the running desktop app (single-instance handoff), so an agent can put its result in front of the person for review.
- ChatGPT (chat app): no official local stdio MCP support was confirmed on 9 October 2026; ChatGPT users use Codex signed in with ChatGPT. A remote connector would need an internet-reachable endpoint and is out of scope for confidential local documents.

### Wave 3

**E — Export between formats**: HWP⇄HWPX through rhwp with its loss report; DOC/DOCX import and export through LibreOffice (detected at run time, not bundled) plus rhwp HTML import/export. A `convert` operation in the service layer, CLI and MCP, and an "Export as…" command in the desktop. Never on save, always a new file, always a loss report.

**F — Review**: read-only Sol review of the merged diff (IPC, protocol handler, subprocess bridge, preservation), then fixes.

## Rules for every worker

- Read `AGENTS.md`, `README.md`, `docs/COMPATIBILITY.md` and this plan first.
- Readable code: clear names, one statement per line, small functions, comments for non-obvious format facts. Reformat the functions you change; do not reformat unrelated files.
- Add a regression test for each defect. Add a test only for a realistic failure that existing tests miss.
- Never weaken: hash-bound patches, expected-text checks, no overwrite, workspace confinement, no execution of document code, honest warnings.
- Run `python3 scripts/run-tests.py` and `npm run check` before each commit.
- No image inspection (Sol workers cannot see images); the orchestrator does visual checks.

## Resume point

Check `git log --oneline main`, `git worktree list` and each worktree's `work/results/*.json`. Continue with the first task whose result file is missing or has `"done": false`.
