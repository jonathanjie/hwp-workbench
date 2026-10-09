# HWP Workbench implementation guide

This is a private developer alpha, not a finished word processor. Read `README.md`, `docs/TECHNICAL-PLAN.md` and `docs/COMPATIBILITY.md` before modifying behavior.

Preserve the product requirement: full native HWP/HWPX editing and closely matching official-document layout. Do not claim that text extraction, an HTML reconstruction, or a synthetic self-round-trip satisfies that requirement.

All document operations belong in the shared engine/service layer. CLI, MCP and desktop must agree. Keep original bytes and opaque native payloads alongside semantic data. Never silently convert on save or discard unsupported objects. Capability and warning reports must stay honest.

Maintain fail-closed, non-overwriting saves. Validate source hashes, expected text, complete patches and output format. Retain bounded parsing, no-follow workspace access, no document-code execution, and an IPC API with no generic shell/filesystem command.

Run `python3 scripts/run-tests.py` and `npm run check` after changes. Add regression tests for every data-loss or compatibility defect. Public corpus failures, unsupported documents and skipped external tests must be reported as such, never converted to passing self-generated fixtures. `scripts/test-ui.py` is an optional renderer test; it does not test Electron or Hancom.

Do not bundle font files, third-party sample documents, credentials or real user documents. Keep fixture source URLs/checksums/provenance, and review licenses before adding dependencies. `npm install` on a connected machine must generate a reviewed lockfile before release builds.

The immediate next gate is real public-file/Hancom and Apple Silicon qualification. The candidate Rust engines are evaluated in the plan, not integrated dependencies. Do not replace native operations with ProseMirror/HTML serialization without proving native data retention.

`reports/` contains evidence from this handoff only. When rerunning tests, retain clear host/platform information and do not label CI/native checks as passed until they actually execute.
