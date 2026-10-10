# Differential engine tests

The Python engine is the frozen reference. These tests compare its RPC and MCP
contracts with another command. They do not certify Hancom layout or full native
editing. No GUI or LibreOffice application is started by the contract cases.

Run the reference against itself:

```bash
npm run test:differential
```

Run a built Rust binary from this project directory:

```bash
python3 scripts/differential.py \
  --candidate '/absolute/path/to/hwp-workbench' \
  --report /tmp/rust-differential.json
```

The command must accept `rpc --root WORKSPACE` and `mcp --root WORKSPACE`.
`--reference` defaults to `python3 -m hwp_workbench`; the runner sets `PYTHONPATH`
to this project's engine folder. Command strings are split into arguments with
`shlex`; they are never passed to a shell. Use an executable wrapper for custom
environment settings. Each request has a 30-second timeout.

Use `--only rpc-hwpx-save` for one case, or `--cases PATH` for another case file.
Each case prints `PASS`, `DIFF` or `SKIP`, followed by a summary. An unapproved
difference or a failed case expectation returns exit code 1. Missing public files
are skipped with their paths; they are never replaced with synthetic inputs.
Native conversion cases are skipped when the reference reports an unavailable
optional backend. Candidate unavailability is a difference when the reference
can run the case.

## Fixture export

```bash
python3 scripts/export-test-fixtures.py
python3 scripts/export-test-fixtures.py --check
PYTHONPATH=engine:tests python3 -m unittest test_differential -v
```

The exporter records every executed document-builder call in `tests/fixtures.py`
and `tests/convert_fixtures.py`, including nested package and CFB builders. It
runs the existing Python tests, excludes its own rebuild/harness tests, disables
real LibreOffice checks, and stops if a capture test fails. It replaces only the
opaque random payload in the engine fixture test with a fixed byte pattern.
Normal engine randomness remains active. All ZIP member times are fixed by the
builders. No external documents or fonts are included.

`tests/data/manifest.json` is an array of `name`, `builder`, `parameters`, `sha256`
and `purpose` records. Purposes list the test IDs that exercise the combination.
Byte arguments use `{"$bytesZlibBase64": "..."}` to keep large stream maps small.
Object order is retained because package entry order is part of the comparison.
Each committed file is rebuilt in memory and checked for byte equality by one
unit test. Rust tests can read the files directly. Some inputs are deliberately
malformed or generic containers; their extensions do not establish format.
Run `--check` after changing tests to detect new or removed builder combinations.

## Case format

`tests/contract/cases.json` is an array of cases with stable `id`, one-line
`purpose`, `seeds`, optional `setup`, and ordered `steps`. Seed records contain
`source` (under `tests/data`, `fixtures/generated` or `fixtures/public`) and a
relative destination `path`. Public seeds can set `optional: true`.

Steps have a unique `id` and a `kind`:

- `rpc`: a `method` and `args` object.
- `mcp`: a JSON-RPC `message`; all exchanges share one process for that case.
  Notifications do not wait for a response.
- `file`: read `path` and compare its bytes or parsed document/JSON contents.
- `list`: list `path`, including whether the directory exists.
- `write`, `mkdir`, `symlink`, `sparse`: prepare controlled error conditions in
  the temporary workspace. `write` accepts `text` or `json`; `symlink` accepts
  a relative `target`; `sparse` accepts `size`.

`setup` uses the preparation kinds before the first step. File paths are relative
and confined. Each side has its own fresh workspace, with identical seed bytes.
The final complete workspace inventory is compared as well as explicit checks.
Seed source bytes must stay exact unless a case performs an explicit save or
external write. Kept save backups retain an exact byte digest.
File modes, directory presence, symlinks and retained source bytes are checked.

A value `{"$ref": "edit/stdout/patch"}` uses the earlier step's raw result.
Components follow JSON Pointer escaping (`~1` for `/`, `~0` for `~`). RPC results
contain `exitCode`, `stdout` and `stderr`; MCP results are their JSON-RPC envelopes.
`{"$workspace": true}` supplies the current side's absolute workspace path.
`expect` maps result paths to exact values. Expectations prevent two equally
broken engines from passing a success or refusal case.

## Comparison and approved differences

JSON types, keys, array order, error codes and messages are exact. Only known
UTC metadata timestamps, generated proposal/backup filename components, temporary
workspace paths, and hashes derived from actual produced bytes are normalised.
Source hashes and document text remain exact. Reported preview and output digests must match their actual bytes before
normalisation. A fabricated digest is not removed.
MCP text content is parsed and compared as well as `structuredContent`.

ZIP documents compare member names, order, compression methods and decompressed
bytes. HWP CFB documents compare stream names and bytes, after raw-deflate decoding
of compressed DocInfo, body and script streams. Opaque streams remain exact. ZIP inflation and HWP deflate/trailer parsing use
the frozen engine bounds (32 MiB per part and 128 MiB total expansion).
Allocation structures, ZIP timestamps and deflate encodings are outside this
semantic comparison. Invalid documents compare their raw bytes. Preview bytes
are also compared as documents. Produced file, preview and stream hashes use a digest of their semantic
representation, rather than an unchecked placeholder. Multiple generated names
keep distinct identities in creation order; folder listings and proposal lists
are sorted after name normalisation.

`tests/contract/approved-differences.json` starts empty. Each approval must name
an exact `case` and JSON Pointer `path`, plus a nonempty `reason` and `approvedBy`.
Approvals do not cover descendants or hide case execution failures. The console
shows up to three readable differences; `--report` retains every difference and
host/Python information. Do not approve a safety or data-retention regression
without review.


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
Hancom reopen, native layout, Electron, or a release build.

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
