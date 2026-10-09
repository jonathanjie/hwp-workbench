# Project lessons

### Use Unicode text for rhwp evidence
- **Date:** 2026-10-09
- **Trigger:** The independent probe used `getTextFileText()` and reported an em dash as `&#8212;`, even though the saved XML contained the correct UTF-8 text.
- **Rule:** Use `getTextFileUnicode()` and decode its JSON string when comparing native edited text. Keep reopen/render evidence separate from Hancom layout qualification. Record corpus checks that cannot run as skipped with the exact reason.
- **Why:** rhwp's TEXT API follows CP949 export rules and escapes characters outside that encoding.

### Use native table positions for form edits
- **Date:** 2026-10-09
- **Trigger:** The HWPX preview omitted cell addresses and clamped spans. Form filling could choose the wrong column or fail to find the cell below.
- **Rule:** Keep native cell addresses and spans in view metadata. Use them for mutation targets, reject ambiguous mappings, and test sparse cells and spans beyond preview limits.
- **Why:** A layout approximation does not supply reliable native edit coordinates.
