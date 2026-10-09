### Use Unicode text and correct scope for rhwp evidence
- **Date:** 2026-10-09
- **Trigger:** The independent probe used `getTextFileText()` and reported an em dash as `&#8212;`, even though the saved XML contained the correct UTF-8 text.
- **Rule:** Use `getTextFileUnicode()` and decode its JSON string when comparing native edited text. Use `getTextRange()` with Unicode scalar offsets for body anchors; its top-level paragraph API omits cell and other nested text, so a missing result does not establish data loss. Keep reopen/render evidence separate from Hancom layout qualification. Record corpus checks that cannot run as skipped with the exact reason.
- **Why:** rhwp's TEXT API follows CP949 export rules and escapes characters outside that encoding.

### Isolate optional dependencies in availability tests
- **Date:** 2026-10-09
- **Trigger:** A missing-package test failed when Node was also unavailable because the runtime check returned first.
- **Rule:** Keep the other dependency available when testing one missing dependency. Test the fully unavailable path separately and retain explicit integration-test skip reasons.
- **Why:** Availability checks run in order; an unrelated missing prerequisite can hide the condition under test.

### Retain native control details for anchors
- **Date:** 2026-10-09
- **Trigger:** Anchor offsets counted paired HWPX controls that rhwp ignores, and HWP table captions were classified as cells.
- **Rule:** Retain XML self-closing status for rhwp inline-control comparisons; auto-number controls inside `hp:ctrl` count in both XML forms. Distinguish HWP caption lists before the direct TABLE record from cell lists after it; table ancestry alone does not identify cell text.
- **Why:** A semantic XML tree and record ancestry omit details that rhwp uses to build text and object structure.
