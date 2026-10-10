# Rust engine and Tauri shell rewrite

Status: cut-over implemented on 2026-10-10; original plan approved by the product owner on 2026-10-10 ("yes rewrite in rhwp; use multiple Sol subagents in parallel").
Branch: `feat/rust` (worktree `hwp-workbench-worktrees/rust`), based on `feat/desktop-ux` 63096d4.

## Goal

Ship HWP Workbench as "our code + rhwp" only: one Rust engine that links rhwp natively, and a Tauri 2 shell that uses the macOS WebKit view. Remove Electron (317 MB) and the bundled Python (60 MB) from the shipped app. Keep every current safety rule and every agent contract.

Targets, measured on Apple Silicon at the cut-over gate:

| Measure | Electron + Python (2026-10-09) | Target |
|---|---|---|
| App bundle | 391 MB | ≤ 60 MB |
| DMG | 182 MB | ≤ 30 MB |
| Engine call latency (inspect, 6-page HWPX) | one Python process per call | in-process, ≤ 50 ms warm |
| Idle memory, one document open | not measured | measured and reported |

This plan preserves rewrite history below. The C1 cut-over makes Rust canonical; retired implementations and commands are no longer available. Current checks are in `docs/TESTING.md`, with C1 execution details in `work/results/C1.json`.

## Decisions

1. **Our engine owns edits and saves. rhwp reads, renders, verifies and exports.** rhwp's own serializer rewrites the whole document; our engine changes only the edited records and keeps every other byte. rhwp replaces the Node bridge for the independent reopen check, page SVG rendering and the conversion export backend. It is never the save path.
2. **One contract, two implementations during the port.** The Python engine is frozen and stays in the repository as the reference oracle until the cut-over gate. A black-box differential runner sends the same RPC requests to both engines. After cut-over we record its outputs as golden files and delete the Python engine.
3. **The renderer keeps its `window.hwp` API.** A small shim implements it with Tauri `invoke` and events. Renderer modules do not change for the shell swap. The renderer keeps rhwp WASM for page display in v1; rendering pages in the backend is a later size optimization.
4. **Thin Tauri glue.** Desktop logic (document session, recents, drafts, agent inbox, preview lane, path checks) lives in `crates/app-core`, which has no Tauri dependency. Tauri code only wires windows, menus, dialogs, single instance and file-open events.
5. **One binary.** The app binary also runs the CLI and the MCP server (`hwp-workbench <command>`), so the app does not carry a second copy of rhwp. `scripts/hwp-workbench` and the MCP bundle call it.
6. **Regex stays restricted.** The engine accepts the same regex subset as today (no groups, repetition, branches or backreferences) and uses the linear-time `regex` crate.
7. **No new runtime downloads for users.** Users install one app. rhwp and all crates compile into it.

## Crates

| Path | Owns | Python source |
|---|---|---|
| `crates/hwp-core` | errors, limits, XML and ZIP bounds, no-follow workspace, CFB, HWP records, document model, format readout | `errors`, `xml_io`, `package_io`, `workspace`, `cfb`, `hwp_records`, `document`, `format_readout` |
| `crates/hwp-verify` | rhwp probe, render, export, `check_output` | `independent`, `engine/bridge/rhwp-bridge.mjs` |
| `crates/hwp-convert` | DOCX and HWPX mappers, loss report, LibreOffice import | `convert/*` |
| `crates/hwp-workbench` | service, agent ops, agent link, skill install, MCP config, CLI, MCP server, `hwp-workbench` binary | `service`, `agent_ops`, `agent_link`, `skill_install`, `mcp_config`, `desktop_open`, `cli`, `mcp` |
| `crates/app-core` | desktop backend logic | `desktop/main.cjs` helpers, `agent-inbox.cjs`, `recent-store.cjs`, `preview-lane.cjs`, `agent-files.py` |
| `crates/app-testhost` | development-only HTTP host for WebKit end-to-end tests | `scripts/test-electron.mjs` harness role |
| `app/src-tauri` | Tauri glue | `desktop/main.cjs`, `menu.cjs`, `preload.cjs`, `protocol.cjs` |

Dependency direction: `hwp-core` ← `hwp-verify` ← `hwp-convert` ← `hwp-workbench` ← `app-core` ← (`app/src-tauri`, `app-testhost`).

## Work packages

Each package is one Sol worker in its own worktree, with a brief in `work/briefs/R*.md` and a result file in `work/results/R*.json`. Packages in the same wave own different paths.

| Wave | ID | Package | Owns | Depends on |
|---|---|---|---|---|
| 1 | R1 | Engine foundation | `crates/hwp-core` (errors → cfb) | scaffold |
| 1 | R3 | rhwp native | `crates/hwp-verify` | scaffold |
| 1 | R4 | Desktop backend and Tauri shell | `crates/app-core`, `app/`, `desktop/tauri-bridge.js`, `scripts/build-frontend.mjs` | scaffold; uses a temporary Python engine adapter |
| 1 | R5 | Differential runner and fixture export | `scripts/differential.py`, `tests/contract/`, `tests/data/` | scaffold |
| 2 | R2 | Document model | `crates/hwp-core` (hwp_records, document, format_readout) | R1, R5 |
| 2 | R6 | Conversion | `crates/hwp-convert` | R1, R3, R5 |
| 2 | R8 | WebKit end-to-end harness | `crates/app-testhost`, `scripts/test-webkit.mjs`, `desktop/http-bridge.js` | R4 |
| 3 | R7 | Service, agents, CLI, MCP | `crates/hwp-workbench` | R2, R3, R6 |
| 4 | R9 | Integration and packaging | in-process engine in app-core, single-binary CLI mode, `scripts/package-mac.cjs`, MCP bundle, docs | all |

## Gates

| Gate | Check | Owner |
|---|---|---|
| G1 | `cargo test --workspace --offline`, `cargo clippy --workspace --offline -- -D warnings`, `cargo fmt --check` pass | each worker, rechecked by the orchestrator |
| G2 | Differential runner: every case matches the Python engine, or has a named, documented, approved difference | R7, R9, orchestrator |
| G3 | WebKit end-to-end: the 13 Electron workflows pass against `app-testhost` | R8, orchestrator |
| G4 | Native smoke on the packaged app: open by double-click and drop, edit, save in place with backup, second launch hands over the file, CLI and MCP from the bundle | orchestrator |
| G5 | Real-key Korean IME in the Tauri window (CGEvent driver) | orchestrator, with the user's quiet minute |
| G6 | Size and memory measured against the targets above | orchestrator |
| G7 | Hancom reopen of saved files on Windows 한글 | cofounder; not a blocker for the alpha, reported as not done until done |

Python tests (`python3 scripts/run-tests.py`) and `npm run check` keep passing on this branch until cut-over.

## Known risks

| Risk | Control |
|---|---|
| WebKit composition events differ from Chromium (Safari sends `compositionend` before the Enter `keydown`, with `isComposing` false and keyCode 229) | Treat keyCode 229 as composing in `slip.js`; G5 real-key test |
| No WebDriver for WKWebView on macOS | Keep Tauri glue thin; test app-core in Rust; run renderer end-to-end in Playwright WebKit against `app-testhost`; G4 native smoke |
| rhwp is large (about 716,000 lines) and its git history is several GB | Pin one commit; vendor offline; measure binary size in R3 |
| rhwp workspace `[patch]` for `svg2pdf` does not apply to dependents | Patch changes PDF determinism only; we do not export PDF |
| Byte-identical output to Python is not possible (different deflate) | Compare decompressed parts and streams; unchanged ZIP entries are copied raw, which preserves more than today |
| Workers cannot reach the network or write to `~/.cargo` | Shared vendor directory and `work/cargo/cargo-offline.sh` with a per-worktree `CARGO_HOME` |
| rhwp's repository is about 9 GB with samples and fonts; the disk is nearly full | Workers build against a trimmed rhwp library copy (36 MB, `work/cargo/rhwp-0.8.7`) through a machine-local `[patch]`, so `Cargo.lock` records rhwp as a path package. R9 regenerates the lock against the pinned git revision for CI. One shared target folder in `/private/tmp` |
| Five Tauri dependencies (cssparser, cssparser-macros, dtoa-short, selectors, option-ext) are MPL-2.0 | Unmodified use is allowed; R9 lists them in `THIRD_PARTY_NOTICES.md` with source links. All other 410 macOS dependencies are MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0 or CC0 |
| rhwp embeds a 13 KB blank HWP template (`saved/blank2010.hwp`) in library code | It already ships inside `@rhwp/core`; list it with rhwp in the notices. Fonts and large samples in rhwp are test-only and do not enter our binary |
| Workers must not launch GUI apps | Workers run Rust tests and Tauri's mock runtime only; the orchestrator runs WebKit, the app and the IME test |

## Status (2026-10-10, end of day)

R1–R9 are merged into `feat/rust`, with R9 split into R9a (Rust engine inside the app, one binary), R9b (packaging), R9c (Claude Desktop bundle outside the signed app) and R9d (native QA driver).

| Gate | Result |
|---|---|
| G1 | Pass: 524 Rust tests (including the live test host), `fmt`, `clippy -D warnings` |
| G2 | Pass: differential 97/97 for the CLI binary and for the app binary through its `hwp-workbench` symlink |
| G3 | Pass: WebKit 13/13 against `app-testhost` with the Rust engine |
| G4 | Pass, packaged app: open through the macOS open event, edit, save in place with a backup, second file handed to the running instance, clean quit (`npm run qa:native`, 21 checks). Packaged CLI read, replace, save and MCP `tools/list` (13 tools) pass. Not yet: drag-and-drop, the Claude Desktop bundle from the app, first open after download (Gatekeeper) |
| G5 | Pass in the edit slip with real 2-Set Korean keys (7 cases). The Electron-only bare control field does not exist in the app |
| G6 | App 26.3 MB, DMG 12.4 MB. Memory with the sample open: Tauri ≈ 214 MB (app 48, WebContent 141, GPU 19, Networking 6) against Electron ≈ 310 MB (main 49, renderer 102, GPU 152, utility 7; its per-call Python and Node processes not counted). Engine CLI: read ≈ 5–10 ms against ≈ 260 ms (Python), save with verification ≈ 25 ms against ≈ 770 ms |
| G7 | Not done: needs the cofounder's Windows 한글 |

### Follow-ups

1. Review sheet: clicks on the right part of the "I'll check the saved document in Hancom" label do not toggle the box (the box and the left part of the label work). Find the element under those points in the WebKit test host with `elementFromPoint` and fix the overlap.
2. Keyboard: WKWebView's default Tab order skips buttons and checkboxes. R8b gave the review dialog buttons explicit tab stops; check every dialog in the packaged app.
3. Both engines reject an absolute path that goes through a symlinked parent of the workspace (for example `/var` → `/private/var`, `/tmp` → `/private/tmp`), while the same file under its real path works. Agents often pass such paths. Resolve the path's parent folders the same way as the root, without following a symlink at the final component; record the difference from Python as an approved difference.
4. Preview of fonts that rhwp has no width table for (for example Apple SD Gothic Neo, used by our own samples): rhwp gives each Latin letter half the font size and stretches the glyph to fit (`lengthAdjust="spacingAndGlyphs"`), so narrow letters look bold. Check a public document that uses 함초롬바탕/돋움 or 맑은 고딕 (rhwp has tables for these), then decide between measuring missing fonts in the renderer and reporting it upstream. Consider a missing-font warning that points to the free official fonts.
5. Completed at C1 cut-over: workspace/app/CLI/skill/bundle version is `0.2.0-alpha.2`, and conversion labels use "direct mapper". Historical mismatch: the CLI reported `0.1.0-alpha.1` (kept equal to Python for the differential), the app `0.2.0-alpha.1`, and conversion reports still name the "Python direct mapper". Align at cut-over and approve the differences.
6. In-process preview cannot be stopped early; check with a large document whether a dropped preview delays the next command.
7. Install guide: replace the manual Claude Desktop JSON step with the app's bundle button once G4 covers it; test the first open of a downloaded DMG on the cofounder's Mac.
8. Completed at C1 cut-over: removed Electron, the Python engine, their packaging and the differential runner. Frozen authored inputs and model/package snapshots remain as Rust regression evidence; new fixtures are built in Rust.
9. Distribution: Developer ID signing and notarization need an Apple Developer account (user decision).

## Resume point

Check `git log feat/rust`, `work/results/R*.json` and the follow-ups above. The packaged app is built by `HWP_WORKBENCH_CARGO_WRAPPER=<repo>/work/cargo/cargo-offline.sh npm run package:mac` (outputs in `dist-mac/`, refused if they exist) and checked natively with `npm run qa:native` during a quiet minute. Start each new worker worktree from the head of `feat/rust`.
