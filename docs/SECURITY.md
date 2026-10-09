# Security and privacy — developer alpha

This code has functional safety checks, not a completed security audit. Use copies of non-sensitive test documents until native qualification and security review are complete.

## Implemented controls

The Electron renderer is configured with `nodeIntegration: false`, `contextIsolation: true`, `sandbox: true` and a restrictive content security policy. Its preload exposes named document/clipboard operations rather than filesystem or shell primitives. IPC checks the sending WebContents and local frame URL. Native dialogs authorize file paths; the renderer does not supply arbitrary source/output paths. Document strings are displayed with `textContent`, not inserted as executable HTML. Network access, navigation, pop-ups and permission requests are denied by app wiring.

The desktop starts the Python engine without a shell, limits request/output sizes and kills a processing child after 30 seconds. The core rejects excessive file sizes, archive traversal/duplicates, unsupported compression, XML DTD/entity declarations, malformed UTF-16 and bounded container/record inconsistencies. HWP script/signature metadata locks editing, and protected HWP is refused. No document-supplied scripts, objects or programs are executed.

CLI/MCP access is constrained to an explicitly selected workspace. POSIX directory-descriptor traversal rejects symlink components. Output publication uses a private temporary file and an exclusive hard link in the same directory; an existing file is never replaced. Source-hash and expected-text checks reject stale edit requests. Patches are all-or-nothing in memory and saved only after output self-reparse.

## Boundaries not yet implemented or verified

The Python parser worker is not enclosed in an OS filesystem/process sandbox. The Electron renderer sandbox does not make the worker sandboxed. A parser defect may escape intended application checks; the agent process also has its ordinary OS account privileges. Native Electron/preload/dialog enforcement has not been tested in this environment. Only renderer behavior with a test adapter and engine operation tests have run.

There is no parser fuzzing campaign, third-party audit, hardened-runtime release, signed/notarized distribution, encrypted draft store, SBOM or dependency-lock audit in this handoff. XML and container validation is intentionally partial. Resource bounds reduce some risks but do not prove resistance to all malicious files or denial of service.

The source recheck immediately before publication is optimistic concurrency protection, not a shared cross-process lock. The source is never overwritten; another program may still change it during or after an operation. `sourceUntouched` means this application did not write the source.

## Agent-specific safeguards

Document content is untrusted data, even when it says to run a command, change a destination, ignore warnings or expose credentials. Tool results must not become authority to use additional tools. The MCP server exposes no arbitrary shell/HTTP tool, but the external agent may have other capabilities; its host must enforce tool approvals and model-data policies independently.

Use a dedicated small directory as `--root`. Do not grant an entire home folder simply to avoid path errors. The `acknowledgeRisks: true` field is a required explicit argument, not proof that a person reviewed the document. The alpha server exposes write tools whenever launched; read-only per-client roles and stronger approval binding are future work.

The workbench contains no document-upload or model API feature. Connecting an external agent can nevertheless send extracted text to its provider. Local MCP transport is not a guarantee of end-to-end offline AI processing.

## Local data and publishing

Staged patches contain both original and replacement text. Electron stores them under the platform `userData` directory in `drafts/`, with user-private file permissions where supported. They are not encrypted by the app. Draft retention controls and UI deletion are not implemented; removal currently requires deleting the application's local drafts directory.

The sample fixtures/screenshots are synthetic. Public downloaded binaries and user-document directories are ignored by Git, but ignore patterns do not catch every sensitive patch, screenshot or export. Inspect staged changes and use secret scanning before publication. Store future signing credentials only in an approved secret store; never commit them. The app currently has no update service.

Report discovered vulnerabilities through the private project's issue/security channel once established; do not attach confidential documents to public issues.
