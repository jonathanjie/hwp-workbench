# Project lessons

### Use Unicode text and correct scope for rhwp evidence
- **Date:** 2026-10-09
- **Trigger:** The independent probe used `getTextFileText()` and reported an em dash as `&#8212;`, even though the saved XML contained the correct UTF-8 text.
- **Rule:** Use `getTextFileUnicode()` and decode its JSON string when comparing native edited text. Use `getTextRange()` with Unicode scalar offsets for body anchors; its top-level paragraph API omits cell and other nested text, so fall back to decoded full Unicode text and report the search method for each edit. A missing top-level result does not establish data loss. Never confirm a deletion with empty-substring search; report it as independently unverified unless the exact target paragraph is checked. Keep reopen/render evidence separate from Hancom layout qualification. Record corpus checks that cannot run as skipped with the exact reason.
- **Why:** rhwp's TEXT API follows CP949 export rules and escapes characters outside that encoding.

### Use native table positions for form edits
- **Date:** 2026-10-09
- **Trigger:** The HWPX preview omitted cell addresses and clamped spans. Form filling could choose the wrong column or fail to find the cell below.
- **Rule:** Keep native cell addresses and spans in view metadata. Use them for mutation targets, reject ambiguous mappings, and test sparse cells and spans beyond preview limits.
- **Why:** A layout approximation does not supply reliable native edit coordinates.

### Keep tests independent of local-only folders
- **Date:** 2026-10-09
- **Trigger:** A corpus test created its temp folder under the gitignored `work/` folder. It passed locally and failed in every clean GitHub Actions checkout; fail-fast then cancelled the macOS jobs.
- **Rule:** Tests use the system temp folder or create what they need. Before every merge to `main`, run the suite in a clean checkout (`git worktree add <temp> HEAD`, run `python3 scripts/run-tests.py` there, then remove it); do not move shared folders aside, because worker worktrees link to `node_modules`. Check that missing optional tools produce skips with reasons. Gate commits on the test command's exit status, not on `grep` output.
- **Recurrence (2026-10-09):** Three more tests from a parallel branch used `ROOT / 'work'`; the merge was pushed without the clean-checkout run and CI failed on all four jobs.
- **Follow-up (B3, 2026-10-09):** A scratch checkout exposed the same assumption in independent-save and missing-parser test helpers. Use system temp folders there too. Build and extract bundle tests in system temp, then test server startup without checkout dependencies.
- **Follow-up (E, 2026-10-09):** Independent-parser service checks and corpus helpers also used `work/`; they now use system temp folders.
- **Recurrence (P merge, 2026-10-10):** A chained command ran the full suite, ignored its failure and pushed `main`. Use `cmd; R=$?; [ $R -eq 0 ] && git push`, never `cmd; …; git push`.
- **Why:** CI runs a clean checkout; local scratch state hides missing setup.

### Resolve agent references before validation
- **Date:** 2026-10-09
- **Trigger:** B1 read printed short references, but edits accepted only full native IDs.
- **Rule:** Use one source-scoped reference map for reads and mutations. Resolve references before duplicate-target and expected-text checks; emit native IDs in generated patches.
- **Why:** CLI conveniences must use the same safety checks as the shared engine.

### Clean up only temporary files created by this write
- **Date:** 2026-10-09
- **Trigger:** An exclusive temporary-name collision reached cleanup and removed the existing temporary file.
- **Rule:** Track successful temporary-file creation and unlink only that owned file. Keep a regression case that forces a collision and checks the existing bytes. Keep recovery-file cleanup inside the replacement rollback guard; an injected cleanup failure must restore the source before the save returns an error.
- **Why:** A failed no-overwrite write must not delete another file during cleanup.

### Match the complete proposal filename
- **Date:** 2026-10-09
- **Trigger:** A proposal for `form.hwpx.other.hwpx` appeared in the inbox for `form.hwpx`.
- **Rule:** Match the complete document name plus the contract's UTC timestamp and hex suffix before selecting up to 200 proposal files.
- **Why:** A shared filename prefix does not identify the same document.

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

### Keep real LibreOffice checks opt-in
- **Date:** 2026-10-09
- **Trigger:** Headless LibreOffice launches in the restricted macOS worker crashed during application registration and showed crash dialogs to the user.
- **Rule:** Test and validation scripts must not start real LibreOffice unless `HWP_WORKBENCH_LIBREOFFICE_TESTS=1`. Normal suites skip those checks with the exact opt-in reason; mocked subprocess tests remain active. Run external qualification outside the restricted worker and record failures separately. Do not retry an installed GUI runtime after an environment-level crash.
- **Why:** Headless flags do not remove macOS application-registration requirements or user-visible crash dialogs.

### Check conversion semantics beyond paragraph text
- **Date:** 2026-10-09
- **Trigger:** Review found ignored default DOCX styles, restored list numbering after an explicit disable, and omitted HWPX image modifiers without loss counts.
- **Rule:** Test default and inherited formatting, explicit removal of inherited properties, and unsupported image modifiers. Apply supported semantics or add named loss counts; matching paragraph text alone cannot establish conversion fidelity.
- **Why:** Simple direct-format fixtures do not exercise inheritance or unsupported modifiers.

### Check generated packages with a second reader
- **Date:** 2026-10-09
- **Trigger:** The DOCX writer passed every self-read test, but LibreOffice refused all its output ("source file could not be loaded") because ElementTree wrote `ns0:Types` and `ns0:Relationships`. LibreOffice also exits 0 in that case.
- **Rule:** For every new writer, open its output in an independent reader (LibreOffice for DOCX, rhwp for HWPX) and assert the output file exists, not only the exit code. Write OPC package parts with an unprefixed namespace.
- **Why:** Our own reader accepts what our writer emits; namespace-prefix tolerance differs between readers.

### Check the save revision before parsing current disk bytes
- **Date:** 2026-10-10
- **Trigger:** A save failure-injection test changed the source to invalid native bytes; parsing returned `UNSUPPORTED_FORMAT` before the stale revision was identified.
- **Rule:** Read bounded source bytes and compare the save patch's source hash before parsing them. Return `SOURCE_CHANGED` for an external revision change; keep the complete patch and native verification checks before replacement.
- **Why:** A changed file can also become unreadable. Parsing must not hide the save revision conflict.

### Build fixtures deterministically
- **Date:** 2026-10-10
- **Trigger:** A DOCX fixture stamped zip members with the current time; a test that compared a written source with a fresh fixture failed when the run crossed a clock second.
- **Rule:** Fixtures that tests rebuild and compare must not depend on time, randomness or the environment. Set fixed zip member times.
- **Why:** Slow or loaded runs cross time boundaries; the failure looks random.
