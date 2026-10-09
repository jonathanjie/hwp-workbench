### Use Unicode text and correct scope for rhwp evidence
- **Date:** 2026-10-09
- **Trigger:** The independent probe used `getTextFileText()` and reported an em dash as `&#8212;`, even though the saved XML contained the correct UTF-8 text.
- **Rule:** Use `getTextFileUnicode()` and decode its JSON string when comparing native edited text. Use `getTextRange()` with Unicode scalar offsets for body anchors; its top-level paragraph API omits cell and other nested text, so fall back to decoded full Unicode text and report the search method for each edit. A missing top-level result does not establish data loss. Never confirm a deletion with empty-substring search; report it as independently unverified unless the exact target paragraph is checked. Keep reopen/render evidence separate from Hancom layout qualification. Record corpus checks that cannot run as skipped with the exact reason.
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

### Verify deleted HWP runs on the native record axis
- **Date:** 2026-10-09
- **Trigger:** Deleting one run can change the segment IDs of later runs when the saved paragraph is parsed again; empty runs also made corpus anchor checks pass without nonempty text evidence.
- **Rule:** Keep source revision IDs stable for patches. Verify complete edited native sections on reopen, including controls and metadata. Emit an existing empty paragraph run for insertion, but exclude empty text from corpus body-anchor evidence. Preserve pre-existing duplicate shape starts; drop only starts that newly collapse during an edit.
- **Why:** Printable-run IDs describe a source revision, and an empty substring alone does not test a reader's text-position agreement.
