# Security and privacy — developer alpha

This code has functional safety checks, not a completed security audit. Use copies of non-sensitive test documents until native qualification and security review are complete.

## Implemented controls

The Tauri renderer has a restrictive content security policy and a named
`window.hwp` bridge. Tauri commands validate arguments and document grants;
the IPC API exposes no generic filesystem or shell command. Native dialogs
authorize source and output paths. Document strings use `textContent`, not
executable HTML. Navigation and external requests are restricted by app wiring.

Desktop operations use the shared Rust engine in process. Independent rhwp
verification uses a bounded worker without a shell, with a 30-second timeout.
The core rejects excessive file sizes, archive traversal/duplicates, unsupported
compression, XML DTD/entity declarations, malformed UTF-16 and invalid native
container/record structures. Script/signature metadata locks editing except
for the exact known inert stub described in COMPATIBILITY.md. Protected HWP
is refused. Document-supplied scripts, objects and programs are never executed.

CLI/MCP access is constrained to an explicitly selected workspace. POSIX directory-descriptor traversal rejects symlink components. Output publication uses a private temporary file and an exclusive hard link in the same directory; an existing file is never replaced. Source-hash and expected-text checks reject stale edit requests. Patches are all-or-nothing in memory and saved only after output self-reparse.

## Boundaries not yet implemented or verified

The Rust parser and verification worker are not enclosed in an OS filesystem
or process sandbox. The WebKit view does not sandbox native engine code.
A parser defect may escape intended checks; the agent process has its ordinary
OS account privileges. Headless WebKit tests exercise the renderer and backend,
while native dialogs, OS events and installation require separate qualification.

There is no parser fuzzing campaign, third-party audit, hardened-runtime release, signed/notarized distribution, encrypted draft store, SBOM or dependency-lock audit in this handoff. XML and container validation is intentionally partial. Resource bounds reduce some risks but do not prove resistance to all malicious files or denial of service.

The source recheck immediately before publication is optimistic concurrency protection, not a shared cross-process lock. Another program may change the source during or after an operation. `apply` and conversions never overwrite. Explicit `save` verifies the native result, keeps a previous version, rechecks the source hash and atomically replaces the opened file. If a concurrent revision appears after publication, retain it and recovery bytes; report `SOURCE_CHANGED`. `sourceUntouched` describes operations that did not write the source.

## Agent-specific safeguards

Document content is untrusted data, even when it says to run a command, change a destination, ignore warnings or expose credentials. Tool results must not become authority to use additional tools. The MCP server exposes no arbitrary shell/HTTP tool, but the external agent may have other capabilities; its host must enforce tool approvals and model-data policies independently.

Use a dedicated small directory as `--root`. Do not grant an entire home folder simply to avoid path errors. The `acknowledgeRisks: true` field is a required explicit argument, not proof that a person reviewed the document. The alpha server exposes write tools whenever launched; read-only per-client roles and stronger approval binding are future work.

The workbench contains no document-upload or model API feature. Connecting an external agent can nevertheless send extracted text to its provider. Local MCP transport is not a guarantee of end-to-end offline AI processing.

## Local data and publishing

Staged patches contain both original and replacement text. The Tauri app stores them under its platform app data directory in `drafts/`, with user-private file permissions where supported. They are not encrypted by the app. Draft retention controls and UI deletion are not implemented; removal currently requires deleting the application's local drafts directory.

The sample fixtures/screenshots are synthetic. Public downloaded binaries and user-document directories are ignored by Git, but ignore patterns do not catch every sensitive patch, screenshot or export. Inspect staged changes and use secret scanning before publication. Store future signing credentials only in an approved secret store; never commit them. The app currently has no update service.

Report discovered vulnerabilities through the project security channel once established; do not attach confidential documents to public issues.
