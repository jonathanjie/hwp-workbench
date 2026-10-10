# Explicit conversion mappers

This crate maps supported DOCX and HWPX features through `model::RichDocument`.
Conversion is lossy. The model's `unsupported` map records named feature counts.
Matching paragraph text does not certify object semantics, pagination or Hancom layout.

Public entry points:

- `read_docx(bytes)` and `read_hwpx(bytes)` return a rich model or `WorkbenchError`.
- `write_docx(model)` and `write_hwpx(model)` return package bytes. They do not publish files.
- `mapper_report`, `merge_unsupported` and `text_check` provide the current service report contract.
- `libreoffice::convert` performs explicit optional legacy import in a private profile.
  Normal tests mock the process boundary and never launch LibreOffice.

R7 owns route selection, source revision checks, risk acknowledgement and exclusive
publication through the shared workspace. R2 owns the complete native document model.
`hwp-verify` remains the native HWP/HWPX export and independent verification backend.
Native same-format saves must not call these conversion writers.

ZIP inflation and XML parsing use `hwp-core`. Output has fixed 2026-01-01 member
timestamps, with an uncompressed first `mimetype` member for HWPX. DOCX OPC parts
use unprefixed default namespaces. Style indexes use canonical immutable value keys.

Tests include exported fixture inputs, 35 frozen Python rich-model/error snapshots,
exact loss counts, layout properties, text-report alignment, a native XML readback,
and rhwp reopen/render checks. These are authored tests. Public-file and Hancom
qualification remain separate gates. Real LibreOffice checks remain opt-in.

Safe differences from the Python oracle are recorded in `work/results/R6.json`:
overlapping DOCX cells are refused, copied image/style allocations are bounded,
output control expansion is checked before XML object allocation, and oversized
HWPX border values are refused instead of reduced. Page-break splitting copies
only the properties that survive the break; it does not copy discarded runs or
numbering. Generated lockfile changes contain only vendored workspace dependencies.
