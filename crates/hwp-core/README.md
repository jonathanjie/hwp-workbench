# Native document core (R1 + R2)

This crate reads native HWP 5 and HWPX, stages complete hash-bound text patches,
and serializes same-format edits while retaining opaque payloads. CLI, MCP,
service orchestration and desktop integration belong to later packages.

| Module | Caller contract |
| --- | --- |
| `document` | `Document::new(bytes, name)` detects native format from bytes. `inspect` and `diff` retain Python JSON key and array order. `patch(&Value)` validates the full batch before mutation, resolves source-scoped short references, and checks source hash and expected text. `serialize(acknowledge_risks)` returns exact original bytes for a no-op; edited output requires acknowledgement, native reopen and payload verification. `node`, `part` and `current_text` provide read access. `patch_input` unwraps the preview envelope. |
| `hwp_records` | Parse bounded native records, preserve exact untouched record bytes, validate paragraph/control ranges, and update only edited text and dependent UTF-16 metadata. At collapsed shape boundaries, preserve duplicate groups and the last original start for surviving content. |
| `format_readout` | `read_format(&Document, node_id)` reports native header properties; absent/unreadable values stay unknown. It does not change document bytes. |
| `errors` | Shared `Result<T>`, `WorkbenchError`, `require` and parsing limits. `as_json` emits the Python error contract. Optional `details` is internal context. |
| `xml_io` | UTF-8 restricted XML trees and lexical tokens retain namespace names, ordered attributes, text/tails, original byte offsets and self-closing form. All XML, HPF and relationship package parts are checked, including unsupported parts. |
| `package_io` | `ZipPackage` validates actual bounded inflation, CRC, completion, lengths and paths. Its writer copies untouched local entry records raw, retaining order, extras, descriptors and comments. |
| `cfb` | `CompoundFile` retains original bytes, directory records and streams. `ordered_streams` supplies Python traversal order; rewrite accepts existing stream names and verifies untouched payloads. |
| `workspace` | Approved canonical root, no-follow descriptor walks, bounded regular-file reads, exclusive new writes, and per-source previous-version retention. Save replacement checks hash, mode and inode, syncs and reads back output, and keeps recovery on conflicts. The service must still verify the complete native patch and keep a previous-version backup before replacement. |

Atomic exchange remains the preferred publication and rollback operation. If a
volume returns `ENOTSUP` or `EINVAL`, replacement uses a plain rename after the
same source hash/inode or output ownership checks. That fallback cannot capture
an external revision written between the final check and rename. Recovery
copies remain valid after rollback sync failures. Open-descriptor writes are
not frozen by either rename form; this is not a cross-process locking protocol.
The fault seams exist only in test builds.

Tests include exported authored fixtures, all-part XML policy, native rhwp
anchor/range and render checks, and the public files when present. Missing,
malformed, protected and unsupported cases are reported separately from
successful edits. No test here qualifies Hancom layout or font fidelity.

The required build wrapper supplies offline vendored dependencies. The only
R2 dependency addition is native rhwp as a development dependency, used by
independent reader tests rather than the preservation writer.
