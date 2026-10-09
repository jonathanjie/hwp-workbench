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

### Keep conveniences from blocking core actions
- **Date:** 2026-10-09
- **Trigger:** The recent-files store refused any profile path with a symlinked parent (macOS `/var`, `/tmp`). Because `load()` awaited it, no document could open, and the welcome screen showed "Recent files unavailable".
- **Rule:** Record recents, thumbnails and similar conveniences best-effort; report their failure without blocking open or save. For no-follow checks, refuse a symlinked leaf (`lstat`), not symlinked parent folders, and continue with the canonical path.
- **Why:** Parent symlinks are normal on macOS; the security goal is not to follow a link the user did not choose.

### Wait for a dialog before `check()` in Playwright
- **Date:** 2026-10-09
- **Trigger:** The second save test called `check()` on the acknowledgement box while the closed dialog still held the earlier tick. `check()` returned at once; the dialog then opened and reset the box, so "Save copy" stayed disabled.
- **Rule:** Wait for the dialog's open state before `check()` or `fill()` on its controls.
- **Why:** `check()` is a no-op for a checked box, even an invisible one.
