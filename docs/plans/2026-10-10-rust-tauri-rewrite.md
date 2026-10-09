# Rust engine and Tauri shell rewrite

Status: approved by the product owner on 2026-10-10 ("yes rewrite in rhwp; use multiple Sol subagents in parallel").
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

## Resume point

Check `git log feat/rust`, `work/results/R*.json` and the job list (`codex-companion.mjs status`). Continue with the first package whose result file is missing or has `"done": false`. Never start a package before its dependencies are merged into `feat/rust`.
