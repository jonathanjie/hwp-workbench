# HWP Workbench

A Korean/English HWP and HWPX document workbench for macOS. One Rust engine, built on the [rhwp](https://github.com/edwardkim/rhwp) library, runs the Tauri desktop app, a command-line tool and an MCP server for AI agents.

**Version 0.2.0-alpha.2 — developer alpha, not a finished HWP editor.** Saved files have not yet been checked in Hancom 한글. Check every saved document in 한글 before you submit it officially.

The product requirement remains full HWP/HWPX editing with close official-document fidelity. This build proves a narrower, preservation-first editing path: it changes existing text and keeps everything else in the file as it was.

## What works

| Area | This build |
|---|---|
| Open | HWP 5.x and HWPX. Pages are drawn by rhwp; English/Korean interface; outline and search. |
| Edit | Change existing text, including table-cell text, with real Korean input (2-Set 두벌식 tested with real key events). Staged changes, undo and redo, change review. |
| Save | Save replaces the opened file only after native verification, atomically, and keeps the previous version in `<document folder>/.hwp-workbench/backups/`. It refuses a file that changed on disk. Also works on exFAT/FAT USB drives. |
| Convert | Explicit HWP/HWPX/DOCX conversion with a loss report. Conversions never overwrite. |
| Agents | The same engine as a CLI and an MCP server (13 tools); an installable Claude Code/Codex skill. |
| Keys | 한글 2022 default shortcuts (checked against 한글's own map) or Mac shortcuts. |

Not yet: new tables, images or general formatting; Hancom layout certification; notarized distribution.

## Install the Mac app

Apple Silicon Mac, macOS 12 or later. Follow [docs/INSTALL-mac.md](docs/INSTALL-mac.md) (Korean, then English). The app is signed ad hoc, not notarized, so the first open needs **System Settings → Privacy & Security → Open Anyway**.

## Build from source

Prerequisites: Apple Silicon Mac, Xcode Command Line Tools (`xcode-select --install`), Rust 1.85 or newer ([rustup](https://rustup.rs)), Node.js 22 or newer.

```bash
git clone https://github.com/jonathanjie/hwp-workbench.git
cd hwp-workbench
npm install
npm run package:mac
```

This builds the frontend and the release binary, then writes `dist-mac/HWP Workbench.app` and a DMG (about 26 MB and 12 MB). Existing outputs are refused; delete `dist-mac/` before a rebuild. The rhwp library is vendored in `third_party/rhwp`, so the build downloads only normal crates.io and npm packages.

For development:

```bash
npm run build:frontend                  # renderer into app/dist
cargo run -p hwp-workbench-app          # the desktop app
cargo run -p hwp-workbench -- --help    # the CLI
```

## Command line

In the installed app the CLI is `/Applications/HWP Workbench.app/Contents/Resources/bin/hwp-workbench`. Add that folder to `PATH`, or use `cargo run -p hwp-workbench --` from a checkout. Paths must be inside `--root` (default: the current folder).

```bash
hwp-workbench read review-form.hwpx --format text
hwp-workbench search review-form.hwpx --query "검토"
hwp-workbench replace review-form.hwpx --find "검토 중" --replace-with "검토 완료" > preview.json
hwp-workbench save review-form.hwpx --patch preview.json --acknowledge-risks
hwp-workbench replace review-form.hwpx --find "검토 중" --replace-with "검토 완료" \
  -o review-form-done.hwpx --acknowledge-risks
hwp-workbench convert input.hwpx --to docx -o exported.docx --acknowledge-risks
```

`replace` without `-o` previews the change and writes nothing. `save` replaces the source after verification and keeps a backup; `apply`, `replace -o` and `convert` write new files and refuse existing ones. Output is JSON unless `--format text` is given; errors are JSON on stderr with exit code 2. See [docs/AGENT-INTERFACE.md](docs/AGENT-INTERFACE.md) for the patch format and every refusal code.

## Use with Claude Code, Codex or Claude Desktop

Keep working in your own agent client; the app is the visual review surface.

```bash
hwp-workbench skill install                                          # Claude Code and Codex skill
hwp-workbench mcp-config --client claude-code --root /absolute/document-folder
hwp-workbench mcp-config --client codex --root /absolute/document-folder
```

`mcp-config` only prints the command to run; it never edits client settings. `/` and the whole home folder are refused as roots. For Claude Desktop, the app's Agents panel builds `hwp-workbench-<version>.mcpb` in `~/Library/Application Support/io.github.jonathanjie.hwp-workbench/` (not yet verified in Claude Desktop).

MCP tools: `hwp_read`, `hwp_search`, `hwp_replace`, `hwp_fill`, `hwp_set`, `hwp_inspect`, `hwp_extract`, `hwp_validate`, `hwp_plan`, `hwp_apply`, `hwp_propose`, `hwp_session`, `hwp_convert`. The server uses stdio only and opens no network listener. Grant the smallest practical folder: an agent host can send extracted text to its model provider, although this app uploads nothing.

## Repository layout

| Path | Contents |
|---|---|
| `crates/hwp-core` | Errors, XML and ZIP I/O, CFB, HWP records, document model, safe workspace and save |
| `crates/hwp-verify` | rhwp verification in a separate worker process |
| `crates/hwp-convert` | DOCX/HWPX mappers and loss reports |
| `crates/hwp-workbench` | Service, agent operations, CLI, MCP server, skill and bundle tools |
| `crates/app-core` | Desktop backend shared by the Tauri app and the WebKit test host |
| `app/src-tauri` | Tauri shell; the app binary is also the CLI (`Resources/bin/hwp-workbench`) |
| `desktop/` | The renderer (HTML/CSS/JS) |
| `third_party/rhwp` | Vendored rhwp 0.8.7 library (MIT) |
| `engine/`, `desktop/main.cjs` | Python engine and Electron shell: the reference implementation for differential tests |

## Tests

```bash
cargo test --workspace                       # Rust engine, app backend, CLI and MCP
cargo clippy --workspace --all-targets -- -D warnings
npm run check && npm run test:desktop        # renderer unit tests
npm run test:webkit                          # 13 app workflows in WebKit against the Rust backend
npm run test:packaging                       # packaging and licence notices
python3 scripts/run-tests.py                 # Python reference engine
python3 scripts/differential.py --candidate target/debug/hwp-workbench   # Rust against Python, 97 cases
npm run qa:native                            # packaged app with real keys; nobody may type during the run
```

Results, gates and open follow-ups are in [docs/plans/2026-10-10-rust-tauri-rewrite.md](docs/plans/2026-10-10-rust-tauri-rewrite.md). Contributors and coding agents: read [AGENTS.md](AGENTS.md) and [tasks/lessons.md](tasks/lessons.md) first.

The bundled sample documents in `fixtures/generated/` are self-authored test files, not real or certified government forms. Public sample documents are listed in `fixtures/public-manifest.json` and downloaded on demand; they are not redistributed.

## Licence

No open-source licence has been granted for this project's own code yet; all rights are reserved. Third-party components and their licences are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and the packaged app carries generated notices for every bundled dependency.
