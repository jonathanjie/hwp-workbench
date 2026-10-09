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

### Keep tests independent of local-only folders
- **Date:** 2026-10-09
- **Trigger:** A corpus test created its temp folder under the gitignored `work/` folder. It passed locally and failed in every clean GitHub Actions checkout; fail-fast then cancelled the macOS jobs.
- **Rule:** Tests use the system temp folder or create what they need. Before merging, run the suite once with local-only folders (`work/`, `fixtures/public/`, `node_modules/`) absent or moved aside, and check that missing optional tools produce skips with reasons.
- **Why:** CI runs a clean checkout; local scratch state hides missing setup.

### Resolve agent references before validation
- **Date:** 2026-10-09
- **Trigger:** B1 read printed short references, but edits accepted only full native IDs.
- **Rule:** Use one source-scoped reference map for reads and mutations. Resolve references before duplicate-target and expected-text checks; emit native IDs in generated patches.
- **Why:** CLI conveniences must use the same safety checks as the shared engine.

### Clean up only temporary files created by this write
- **Date:** 2026-10-09
- **Trigger:** An exclusive temporary-name collision reached cleanup and removed the existing temporary file.
- **Rule:** Track successful temporary-file creation and unlink only that owned file. Keep a regression case that forces a collision and checks the existing bytes.
- **Why:** A failed no-overwrite write must not delete another file during cleanup.

### Match the complete proposal filename
- **Date:** 2026-10-09
- **Trigger:** A proposal for `form.hwpx.other.hwpx` appeared in the inbox for `form.hwpx`.
- **Rule:** Match the complete document name plus the contract's UTC timestamp and hex suffix before selecting up to 200 proposal files.
- **Why:** A shared filename prefix does not identify the same document.
