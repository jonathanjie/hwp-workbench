# Compatibility contract — alpha 0.1

The initial engine is a preservation experiment, not a Hancom-compatible word processor certification. Always inspect its warnings before edits and independently check output before use.

## Supported boundaries

HWP support starts at 5.x, stored in a CFB version 3 or 4 container. Compressed and uncompressed paragraph streams are parsed. Only contiguous existing text can be replaced, and replacements must have the same UTF-16 code-unit length. Paragraph structure, style references and native controls are not editable. All selected segments in one batch must pass validation before anything is changed.

HWPX support assumes a ZIP package with `application/hwp+zip` mimetype, `Contents/header.xml`, `Contents/content.hpf` and numbered `Contents/sectionN.xml` bodies. XML must be UTF-8 and well formed, without DTDs or entity declarations. Plain text in namespace-qualified `hp:t` nodes can be changed, including an existing empty node. Text containing child controls is read-only. Edits do not add or remove paragraphs or text-node identities.

The format extension never substitutes for detection. Output must use the same native extension as the detected source. There is no implicit HWP/HWPX conversion, and no HTML/DOCX conversion on save.

## What preservation actually means

No-op serialization returns the original bytes. Edited HWPX output retains the uncompressed bytes of untouched members and changes text lexically within affected XML members; ZIP compression/container bytes may change. Edited HWP output retains unaffected stream payloads but rebuilds CFB allocation structures and recompresses edited sections. The writer verifies member/stream inventories, intended text values and untouched payloads with its own reader before output publication.

This does not imply visual fidelity. Cached preview text, images and line positions may still describe the original content. HWPX length changes can leave text-position-dependent metadata unadjusted. HWP equal-length replacements can change visual width. Fonts, pagination, line breaking, object anchors and field/revision semantics are not independently qualified. The safe operating rule is a new output file followed by Hancom comparison, not blind submission.

## Protected, active and unknown content

Protected/DRM/password HWP is refused. Detected script or signature metadata blocks edits. The one exception is Hancom's default script stub (`Scripts/DefaultJScript` with an empty `OnDocument_New` and `Scripts/JScriptVersion`): when both decompressed payloads match the recorded SHA-256 hashes exactly, the document stays editable, the streams are kept byte-identical and never executed, and an `info` warning `DEFAULT_SCRIPT_STUB` is reported. Any other script content keeps the document read-only. Signed documents are not silently invalidated or stripped. The app does not execute macros, scripts, OLE or external references.

Known feature warnings cover tables, images, equations, charts, OLE, headers/footers, fields, footnotes and selected shapes. Unknown payloads are retained where possible. Detection of every unsupported feature is not implemented, so the general uncertified-layout warning is never removed. A missing feature-specific warning is not a claim of support.

HWPX preview is an approximate structure view; HWP preview is structured text. Table cells with editable text are not equivalent to a table editor. Existing styles may be displayed but there is no formatting toolbar because native formatting mutation is not implemented.

## Limits

Files and saved outputs: 64 MiB. One expanded part: 32 MiB. Total ZIP expansion or HWP body expansion: 128 MiB. ZIP members: 2,048. Text nodes: 100,000. Text operations per patch: 1,000. Text value length: 200,000 Unicode characters. Section XML depth: 128. Desktop subprocess timeout: 30 seconds. MCP input message length: approximately 4 MiB characters; this is not a full resource-isolation guarantee.

`validate` means basic container/XML/record parsing succeeded. `reopenedAndVerified` means our own reader verified the output, not that Hancom or an independent parser accepted it. No metric is reported as a percentage of Hancom compatibility.

The `apply` result reports an independent rhwp reopen and edited-text check; a failed or unavailable rhwp check does not block a save that passes the preservation engine’s own verification.

Node `anchor` values use zero-based section and top-level paragraph indexes and Unicode character offsets from rhwp’s paragraph text API. Table-cell text keeps its containing top-level paragraph with `inCell: true` and `charOffset: null`; other nested text, such as shape text and headers, also has a null offset because this API cannot address it. The independent edited-text check searches the probed top-level paragraphs, so it can report nested edited text as missing even when the native save retains it. The corpus report lists these range checks as skipped when a file has no body text. Node rendering uses approximate text widths and does not qualify font metrics or Hancom layout.

## Explicitly not implemented

General HWP length changes; HWP 3.x; complete native rendering or pagination; character/paragraph-format edits; paragraph insertion/deletion; table structural changes; images/stamps/object edits; native fields/form operations; new documents; comments/revisions; password/signature workflows; conversion; PDF/DOCX export; collaborative editing; autosaved unstaged typing; bundled native engine/runtime; release signing and notarization.
