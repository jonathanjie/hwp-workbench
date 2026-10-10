# Frozen authored fixtures

These inputs are frozen regression fixtures for Rust tests. The original
exporter captured document-builder calls and fixed random payloads and ZIP
timestamps. See the deleted `scripts/export-test-fixtures.py` at commit
`6d8f17f` in Git history, together with the builders named in `manifest.json`.

The manifest retains builder names, parameters, SHA-256 and originating test
purposes. The source-app launch test title now names the current app; its
original title is in that commit. No fixture bytes have changed at cut-over.
Rich-model and package snapshots retain evidence recorded before the port.

Build new fixtures in Rust tests. Use system temporary folders for published
outputs. These are self-authored parser and preservation cases, not public
documents or evidence of Hancom acceptance.
