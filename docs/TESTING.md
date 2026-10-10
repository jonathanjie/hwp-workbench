# Rust and renderer tests

Rust is the canonical engine. CLI, MCP and Tauri share the operation layer.
These checks do not certify full editing, Hancom layout or interoperability.
Normal tests do not launch the GUI or real LibreOffice.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
npm ci
npm run build:frontend
cargo test --workspace --locked -- --include-ignored
npm run check
npm run test:desktop
npm run test:packaging
npx playwright install webkit
npm run test:webkit
node --test tests/qa/*.test.mjs
```

`npm test` runs Rust tests and all Node unit suites. To run the ignored
loopback and native file-watcher tests, use the explicit command above.
WebKit runs all 13 renderer workflows against `app-testhost` and the Rust
backend. It uses the HTTP bridge only in a test build; release builds use the
Tauri bridge. `HWP_WORKBENCH_CARGO_WRAPPER` can select a Cargo wrapper; by
default tests call `cargo` and read its target directory from metadata.

`tests/data/` holds frozen authored inputs and their provenance manifest.
Rich-model/error snapshots and mapper package snapshots remain regression
evidence. See `tests/data/README.md`. Build new inputs in Rust tests or examples;
`hwp-core/examples/locked-fixture.rs` builds the renderer search fixture.
A synthetic round-trip alone is not independent compatibility evidence.

Download public fixtures with `node scripts/fetch-public-fixtures.mjs`, then
run `cargo test -p hwp-core --test corpus --locked`. The manifest pins immutable
Git blob hashes and sizes; the downloader records SHA-256, checks it when
supplied, and refuses existing invalid files. Missing public files remain
skipped with their exact paths, never replaced with authored samples.
Keep temporary results in `work/` and final reports in `outputs/`, with host
and platform information. Do not claim CI or native checks passed unless run.

Real LibreOffice tests require `HWP_WORKBENCH_LIBREOFFICE_TESTS=1`; run external
qualification only on a host where it can register without GUI crash dialogs.
The usual mapper tests mock the process boundary. Public-file, native macOS,
Hancom, font/layout and release-signing gates remain separate.

## Native Tauri keyboard and save qualification

Reserve a quiet minute or two. **Nobody may type or click during this run.**
The driver sends real macOS keyboard and mouse events to the packaged app. Build
it first with `npm run package:mac`. Enable 2-Set Korean in macOS input sources,
grant Accessibility permission to the terminal or runner, unlock the screen,
and quit every other HWP Workbench instance. Node 22+ and the Swift compiler
are required. The helper never requests permission or unlocks the screen.

```bash
# Read-only preflight: no launch, source switch, clicks, keys or quit.
npm run qa:native -- --dry-run

# Run only during the reserved quiet period.
npm run qa:native
# Use an existing helper or a package in another checkout:
npm run qa:native -- --app '/absolute/path/HWP Workbench.app' --helper /tmp/ime-keys
```

The default package is `dist-mac/HWP Workbench.app`. The driver copies
`review-form.hwpx` and `legacy-review.hwp` to a new system temporary folder.
It measures window and render readiness, reads values and focus through
Accessibility, tests Korean composition, Backspace and Enter, stages with
Cmd+Enter, and saves with Cmd+S through the app's acknowledgement sheet. The
bundled CLI checks the new text, changed SHA-256 and retained native format. A backup must contain the
exact original bytes. A second `open -a` must display the HWP sample in the same
process. The input source is restored and the app is quit in cleanup; only
recorded app-owned PIDs can receive termination signals if normal quit fails.
The copied files and backups remain at the printed temporary path for review.

The current renderer has no bare `#imeControl` textarea. The driver records
those rounds as required `notTested` checks and exits 1; it does not alter the
bundle or replace the control with a search field. It checks the WebKit
Cmd+Enter outcome, but Accessibility cannot capture composition event order.
Memory uses `footprint -f bytes` for the app and helpers with known ownership,
with separately labelled `ps` RSS readings when footprint is unavailable.
Remote WebContent processes can be identified from the app Accessibility tree.
Other launchd-owned helpers need a usable responsibility record from
`launchctl procinfo`; the driver never uses sudo and excludes helpers with
unknown owners. An incomplete physical footprint is required `notTested`.

Stdout contains one JSON report; progress goes to stderr. The report includes
checks, timings, memory method, host information and `notTested`. Failed checks
or required omissions return exit code 1. A successful dry-run means only that
preflight passed. These synthetic checks do not qualify public documents,
Hancom reopen, native layout or a release build.

Development checks that do not launch the app:

```bash
swiftc -O -warnings-as-errors scripts/qa/ime-keys.swift -o /tmp/ime-keys
node --check scripts/qa/native-tauri.mjs
node --test tests/qa/*.test.mjs
npm run check
```

The helper adds `preflight`, `ax-find <pid> <id>`, `ax-click <pid> <id>`,
`ax-value <pid> <id>`, `ax-focused <pid>`, `ax-processes <pid>` and
`wait-window <pid> <seconds>`.
AX results are JSON; `ax-value` preserves embedded newlines. Button-label
searches (`ax-find-text` and `ax-click-text`) open text hits without DOM IDs.
`guarded-keys <pid> <id-or-dash> <code[:modifier]>...` refuses events if the
screen is locked, the app is not frontmost, or the expected field loses focus.
The existing `current`, `select` and `keys` commands retain their behavior.
