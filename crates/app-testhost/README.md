# Renderer test host

This development binary serves `app/dist/` and calls the same `app-core::Backend`
as Tauri. Release packaging must not include this binary or `http-bridge.js`.

Run through the approved offline Cargo wrapper. The binary accepts exactly two
arguments: the project source root and a private test profile root. It binds
`127.0.0.1` on a random port and prints `{"url": ..., "token": ...}` as its first
stdout line. Use the returned URL as the base for every request. Its random token
path authenticates assets, commands, controls and events. Logs go to stderr.

Build the test frontend with `npm run build:frontend -- --test`. A normal
`npm run build:frontend` excludes the HTTP bridge. `npm run test:webkit` builds
the test assets and host, runs headless WebKit, and restores normal assets during
teardown. Set `HWP_WORKBENCH_CARGO_WRAPPER` if the offline wrapper is elsewhere.

Use `npm run test:webkit -- --list` to list the 13 workflows without starting
any host or browser. Use `npm run test:webkit -- --only ime-keys` to select one.
Screenshots go to `work/qa/webkit/`. The summary separates passed checks,
skipped checks and native or external features that were not tested. WebKit
composition is a replay; it does not qualify real OS Korean input.

All command calls are `POST invoke/<Tauri command>` with the same argument JSON
and result JSON. Errors retain the backend's `code` and `message`. The host uses
the same FIFO command queue and separate preview lane as the native shell.
`GET events` streams `app:command`, `document:handoff`, `agent:inbox` and trusted
`tauri://drag-drop` events through SSE. The bridge waits for stream registration
before it reports handoff readiness.

Test controls use POST and JSON:

| Relative endpoint | Arguments | Effect |
| --- | --- | --- |
| `test/dialog/open` | `{"path":"/absolute/file.hwpx"}` or `{"path":null}` | Queue one open answer. |
| `test/dialog/save` | `{"path":"/absolute/copy.hwpx"}` or `{"path":null}` | Queue one save answer. |
| `test/dialog/message` | `{"answer":1}` | Queue one zero-based button answer. |
| `test/state` | `{}` | Read message questions, revealed paths and copied strings. |
| `test/menu` | `{"command":"undo"}` | Send an allowed renderer menu command. |
| `test/handoff` | `{"path":"/absolute/file.hwpx"}` | Queue a backend file-open handoff. |
| `test/drop` | `{"paths":["/absolute/file.hwpx"]}` | Grant an OS drop, then send its event. |

An empty open/save queue cancels. An empty message queue selects the backend's
safe default. These fakes never launch native dialogs or copy to the OS clipboard.
A drop grant is consumed once by the shared backend.

Run Rust checks with the offline wrapper: `test -p app-testhost` and
`clippy -p app-testhost --all-targets -- -D warnings`. The worker sandbox refuses
loopback binds. Two live HTTP/SSE tests are marked ignored with that reason.
The orchestrator must run `test -p app-testhost -- --include-ignored` on a host
with socket permission. They must not be reported as passed until executed.
