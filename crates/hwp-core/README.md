# Engine foundations (R1)

This crate supplies native container and workspace primitives. It does not yet
supply a document model, patch service, converter, CLI, MCP server or desktop.
Authored fixtures test preservation and bounds. They do not qualify Hancom
layout or public document compatibility.

| Module | Caller contract |
| --- | --- |
| `errors` | Use `Result<T>`, `WorkbenchError`, `require` and the shared limits. `as_json()` returns the error detail; CLI adds its `error` envelope. Optional `details` is internal context and is not added to the Python JSON contract. |
| `xml_io` | `xml_tree` owns namespace names, ordered attributes, text, tails and children. Namespace declarations are separate from ElementTree attributes. `Element::get` accepts expanded names. `TokenReader` validates the same policy and returns original byte ranges, resolved names, preorder indexes and empty-tag form. Empty tags produce a single Start token. Callers supply native text-node selection and edit logic. |
| `package_io` | `ZipPackage` keeps the source, central-directory order and original Python member names. `read_member` takes the prior actual expansion plus part and total budgets; callers increment the prior expansion by the returned length. It checks CRC, length and complete deflate data in 64 KiB chunks. `rewrite` retains untouched local records verbatim and changes only required size, CRC and offset fields elsewhere. It retains opaque extra fields, comments, descriptors and ZIP64 structures. Native package inventory and XML-part dispatch belong to R2/R6. |
| `workspace` | Construct an approved canonical `Workspace`. Path methods use no-follow descriptor walks. `read` uses the standard limit; `read_with_options` permits a narrower budget and absent-file handling. `list_files` takes a full-name predicate and a limit; a caller can supply its own anchored regex without an added core dependency. `write_new` never replaces a destination. `backup_existing` keeps ten versions per canonical source. `replace_existing` checks hash, mode and inode, syncs files and directories, reads back output and retains recovery on conflicts. |
| `cfb` | `CompoundFile` keeps original bytes, raw directory entries and every stream. Use `ordered_streams()` to retain Python traversal order; its public maps provide lookup. `rewrite` accepts existing stream names only, returns original bytes for a no-op and reparses to verify every opaque payload. Integer field helpers return bounded `Result` values. |

The workspace failure hook and deterministic name/clock controls exist only in
unit-test builds. Production callers cannot select or pay for those hooks.

R1 makes two safety changes to Python. Publication and rollback use atomic
exchange on macOS/Linux, so a pathname race retains the exact displaced file.
Unsupported exchange fails closed; there is no overwrite-rename fallback.
Rollback and recovery failures keep original bytes and report a recovery path.
Atomic exchange cannot freeze bytes held by another writer's open descriptor.
Captured revisions are retained when a race is detected; this is not a
cross-process locking protocol. R7 must still create the previous-version
backup and verify complete native patches before calling replacement.

Integration test targets are `errors`, `xml_io`, `package_io` and `cfb`.
Workspace tests are `hwp_core::workspace::tests`. The R1 result maps each Python
case, including inherited save tests, to a target or its R2/R7 deferral.
