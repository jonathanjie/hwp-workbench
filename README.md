# HWP Workbench

Private Korean/English document workbench, with a shared local engine, Electron desktop interface, CLI and stdio MCP server.

**Version: 0.1.0-alpha.1 — developer source build, not a finished HWP editor.**

The product requirement remains full HWP/HWPX editing with close official-document fidelity. This first build proves a narrower preservation-oriented editing path. It is not suitable for unreviewed government submissions. In particular, it does not yet provide native pagination, general formatting tools, or unrestricted HWP editing.

## What is implemented

| Capability | This build |
|---|---|
| HWPX | Open, inspect, extract, edit existing plain text (including table-cell text), save a new HWPX copy. Text length may change. |
| HWP | Open supported HWP 5.x containers, inspect/extract text, make length-changing text edits in existing plain text runs, save a new HWP copy. Hancom reflow is unverified. |
| Conversion | Explicit, reported HWP/HWPX/DOCX export through HWPX. Direct Python DOCX mapping; native format export uses optional rhwp. DOC/ODT/RTF import requires optional LibreOffice. Lossy and unqualified for Hancom layout. |
| Desktop | English/Korean UI; outline/search; selectable structured preview; text inspector; staged undo/redo; change review; local staged-patch recovery; native open/save-dialog wiring. |
| Preservation | Original file never overwritten; untouched payloads retained; changes bound to the source hash and expected text; output reopened by this engine before publication. |
| Agents | Shared native read/search/edit/plan/apply operations, bounded proposals and session reports, plus an installable Claude Code/Codex skill. CLI and MCP use the same engine as the desktop. |
| Packaging | Apple Silicon developer `.app` packaging script and private GitHub publishing script. Neither has been executed on a Mac in this build environment. |

The GUI is a **structure preview**, not a faithful page renderer. HWP tables, images, shapes and page layout are not drawn. HWPX table structure and selected character styles are approximated; image/object payloads have placeholders. Keeping bytes does not guarantee unchanged visual layout.

## Run on Apple Silicon macOS

Prerequisites: **Node.js 22 or newer**, **Python 3.11 or newer**, and internet access for the initial Electron dependency installation. The Python engine itself has no third-party dependencies. The alpha does not bundle Python.

From the extracted project directory:

```bash
./scripts/bootstrap-mac.sh
```

The script checks prerequisites, generates the synthetic examples, runs core tests, installs the pinned Electron dependency, checks JavaScript syntax and starts the app. Subsequent launches:

```bash
npm start
```

An alternate Python installation can be selected explicitly:

```bash
export HWP_WORKBENCH_PYTHON=/absolute/path/to/python3
npm start
```

Click **Open synthetic example** for the demonstration. Select a text node in the page or outline, change its text in the inspector, click **Stage text edit**, then **Review changes**. Saving requires acknowledging compatibility risks and selecting a **new** filename. Existing destinations are refused, even when a platform dialog offers replacement.

Typing is not committed until staged. Staged patches are written to Electron's per-user application-data directory, under `drafts/`. Those files may contain document text; they are not encrypted by this app. A draft is offered for recovery only when the source hash matches. Saving a copy retains the current original-document session and its staged changes; it does not silently switch the session to the new file.

## Use the CLI without installing Electron

Run from the project directory. Paths are constrained to the current directory unless `--root` is supplied. An output's parent directory must already exist.

```bash
./scripts/hwp-workbench capabilities
./scripts/hwp-workbench inspect fixtures/generated/review-form.hwpx
./scripts/hwp-workbench extract fixtures/generated/legacy-review.hwp --format text
./scripts/hwp-workbench validate fixtures/generated/review-form.hwpx

./scripts/hwp-workbench plan fixtures/generated/review-form.hwpx \
  --patch examples/review.patch.json

./scripts/hwp-workbench apply fixtures/generated/review-form.hwpx \
  --patch examples/review.patch.json \
  --output review-form-edited.hwpx \
  --acknowledge-risks
```

Explicit conversion is separate from save. First inspect a dry-run report:

```bash
./scripts/hwp-workbench convert input.docx --to hwpx --root /approved/workspace
./scripts/hwp-workbench convert input.hwpx --to docx --root /approved/workspace \
  -o exported.docx --acknowledge-risks
```

Without **both** output and risk acknowledgement, conversion returns a report and writes no file. Existing outputs are refused. The report lists the route, engine loss reports, unsupported feature counts, and a comparison of readable paragraph text. A zero-loss rhwp report does not cover Python import losses or prove matching layout. Review the complete report and compare the new file in Hancom before official use. See `docs/COMPATIBILITY.md`.

JSON is the default output. Errors are JSON on stderr with exit code 2. `validate` checks bounded parsing and container structure; it is **not** a full HWPX schema or Hancom-rendering validation. Re-running the final command with an existing output fails instead of replacing the file.

A corresponding HWP example is in `examples/legacy.patch.json`:

```bash
./scripts/hwp-workbench apply fixtures/generated/legacy-review.hwp \
  --patch examples/legacy.patch.json \
  --output legacy-review-edited.hwp --acknowledge-risks
```

The bundled documents are self-authored, format-shaped parser fixtures. They contain deliberately opaque test data and are **not certified or intended as complete, submission-ready Hancom documents**. Successfully reopening them in this engine does not prove interoperability with Hancom.

## Use with Claude Code or Codex

```bash
scripts/hwp-workbench skill install
```

This installs the marked skill for both agents. Use `--target claude` or `--target codex` to select one, and `--dry-run` to check destinations without writing. `skill status` shows installed versions and paths; `skill uninstall` removes only marked folders. Existing unmarked skills are preserved, and marked skills are backed up before replacement.

The skill guides `read` → `search` → edit preview → diff review → `propose` for desktop review or `apply` to a new native file. `session` reports Agent link state and recorded decisions. Desktop session publishing and blue-pencil review require the separate desktop integration; this task supplies the engine file contract. Keep hashes, exact expected text and honest compatibility warnings. See `docs/AGENT-INTERFACE.md`.

## Connect an agent through MCP

Use a compatible client supporting the negotiated 2025-11-25, 2025-06-18 or 2025-03-26 profile. This server does not claim conformance to later MCP revisions. It has no HTTP endpoint and opens no network listener.

Example client configuration, replacing both absolute paths:

```json
{
  "mcpServers": {
    "hwp-workbench": {
      "command": "/absolute/path/hwp-workbench/scripts/hwp-workbench",
      "args": ["mcp", "--root", "/absolute/path/approved-document-workspace"]
    }
  }
}
```

Tools are `hwp_read`, `hwp_search`, `hwp_replace`, `hwp_fill`, `hwp_set`, `hwp_inspect`, `hwp_extract`, `hwp_validate`, `hwp_plan`, `hwp_apply`, `hwp_propose`, `hwp_session`, and `hwp_convert`. Start with inspect, use the returned source hash and node IDs to construct a patch, review plan output, then apply to a new filename. A node ID is scoped to its source revision; re-inspect after changing the file.

Grant the smallest practical workspace. An agent host can transmit extracted content to its model provider even though this application does not upload documents. The `acknowledgeRisks` flag acknowledges a policy; it is not cryptographic evidence of human approval. See `docs/AGENT-INTERFACE.md` and `docs/SECURITY.md`.

## Tests and public fixtures

```bash
python3 scripts/run-tests.py
npm run check

# Optional external checks; use a host where LibreOffice can run.
HWP_WORKBENCH_LIBREOFFICE_TESTS=1 PYTHONPATH=engine:tests \
  python3 -m unittest \
  test_convert.ReverseMapperTests.test_libreoffice_independently_opens_output_docx \
  test_convert.LibreOfficeSafetyTests.test_real_rtf_import_when_runtime_can_execute -v

# Requires internet access; only downloads, never uploads document bytes.
python3 scripts/fetch-public-fixtures.py
python3 scripts/test-public-corpus.py
```

At handoff, **68 core/CLI/MCP tests passed** on Linux/Python 3.13.5. **15 renderer workflow checks passed** in Chromium with the real Python engine and a test-only IPC adapter. The optional renderer harness needs Playwright and Chromium: `python3 scripts/test-ui.py`.

Real LibreOffice tests are opt-in. The normal suite skips them with a reason and runs mocked isolation/timeout tests. On this restricted macOS worker, LibreOffice aborted during application registration before document import; no PDF or legacy import success is claimed. Run the opt-in checks outside this worker.

Public fixture metadata was located online for two legacy HWP examples and one HWPX report. Downloads failed here because the execution environment could not resolve external hosts. Consequently **no public binary corpus tests ran**. The manifest pins repository blobs and expected sizes, and the downloader records SHA-256 after verification. Missing samples never count as passing tests. Public document binaries are not redistributed or committed.

Reports and screenshots are under `reports/`. Mac Electron execution, native dialogs, real Korean IME composition, Apple Silicon packaging/signing, and Hancom reopen/layout comparisons have **not** been tested. The CI workflow is provided but has not run on GitHub.

## Private GitHub repository

The connected GitHub account was identified as `jonathanjie`, but no repository was created or pushed from this environment. On your authenticated development machine:

```bash
./scripts/publish-private.sh jonathanjie/hwp-workbench
```

This requires the GitHub CLI (`gh`), authentication and a configured Git author. It creates a **new private** repository and refuses to modify an existing one. Review staged files before publication; ignore patterns are not a secret-scanning guarantee. Supply another `owner/repository` argument to target an organization instead.

There is no npm lockfile in this handoff because dependency resolution could not complete here. The first connected `npm install` produces it. Review and commit the resulting lockfile before treating builds as reproducible.

## Developer application package

After installing dependencies on an Apple Silicon Mac:

```bash
npm run package:mac
```

Intended output: `dist/mac-arm64/HWP Workbench.app`. The script copies the locally installed Electron runtime and application source, then applies an ad-hoc signature. It does **not** produce a notarized release or bundle Python. It refuses an existing build destination. This is an untested developer packaging path; the handoff does not contain a working `.app` or `.dmg` binary.

## Documents

`docs/TECHNICAL-PLAN.md` contains the implementation plan, engine evaluation, milestones and release gates. `docs/COMPATIBILITY.md` defines exactly what preservation means. `docs/AGENT-INTERFACE.md` documents the patch/CLI/MCP contract. `docs/SECURITY.md` describes implemented controls and remaining risks. `AGENTS.md` guides subsequent implementation work.

The next acceptance gate is successful edit/save/reopen on real HWP/HWPX documents using Hancom as an independent reference, not more cosmetic UI features.
