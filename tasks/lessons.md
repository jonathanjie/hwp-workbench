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
- **Recurrence (2026-10-10):** `unittest ... | tail -3 && git commit` committed R5 after the test command reported errors, because `tail` succeeded. The errors came from a wrong import path, not from the code, but the gate did not work. Save `R=$?` from the test command itself (no pipe), and commit only when `$R` is 0. Run single test modules with `PYTHONPATH=engine:tests`, as `scripts/run-tests.py` does.
- **Follow-up (E, 2026-10-09):** Independent-parser service checks and corpus helpers also used `work/`; they now use system temp folders.
- **Recurrence (P merge, 2026-10-10):** A chained command ran the full suite, ignored its failure and pushed `main`. Use `cmd; R=$?; [ $R -eq 0 ] && git push`, never `cmd; …; git push`.
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
- **Date:** 2026-10-10
- **Trigger:** Deleting one run can change the segment IDs of later runs when the saved paragraph is parsed again; empty runs also made corpus anchor checks pass without nonempty text evidence. Keeping the first collapsed shape changed the style of surviving controls and text.
- **Rule:** Keep source revision IDs stable for patches. Verify complete edited native sections on reopen, including controls and metadata. Emit an existing empty paragraph run for insertion, but exclude empty text from corpus body-anchor evidence. At a collapsed shape boundary, keep the last original start so surviving text and controls retain their style. Keep all pre-existing duplicate groups, including earlier groups at a collapsed boundary; the surviving-content shape must remain last.
- **Why:** Printable-run IDs describe a source revision, and an empty substring alone does not test a reader's text-position agreement.

### Test Korean input with real OS key events
- **Date:** 2026-10-09
- **Trigger:** Simulated composition passed, but real 2-Set Korean typing on macOS showed two defects: Enter arrives twice (composing, then plain) and added a line break; ⌘↵ arrives once with `isComposing` set, so the slip ignored it.
- **Rule:** Qualify text entry with `scripts/qa/real-ime.mjs` (real HID key events, a bare control textarea, nobody typing). Record the event sequence and replay it in `scripts/test-electron.mjs`. A request made during composition (such as ⌘↵) runs after `compositionend`.
- **Why:** Synthetic events copy what we expect; the input method decides the real order.

### Pass single-instance requests as lock data
- **Date:** 2026-10-10
- **Trigger:** `--open <file>` from a second launch was ignored: Chromium reordered the argv that `second-instance` receives (switches before paths), so the file no longer followed `--open`.
- **Rule:** The second instance parses its own `process.argv` and passes the request through `app.requestSingleInstanceLock(additionalData)`; the first instance reads `additionalData` and uses argv only as a fallback.
- **Why:** Electron documents that Chromium may add or reorder second-instance arguments.

### Key per-document state by file and content
- **Date:** 2026-10-10
- **Trigger:** Drafts were stored by content hash only, so opening a copied form offered the original file's staged edits.
- **Rule:** Key drafts, sessions and similar state by the canonical file path and the content hash together.
- **Why:** Copies of official forms are common and share identical bytes.

### Wait for effects in Electron tests, not for the click
- **Date:** 2026-10-10
- **Trigger:** Three checks read state right after a click: marks drawn after a preview loads, a reveal queued behind a preview render, and an outcome file written by main. They passed or failed by timing.
- **Rule:** Wait for the observable effect (selector, main-process flag, file) with a deadline. Main serializes IPC requests, so any slow request delays later ones.
- **Why:** Live preview added long-running work to the same queue.

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

### Call browser timer functions unbound
- **Date:** 2026-10-10
- **Trigger:** The keymap stored `setTimeout` as a property and called `this.setTimer(...)`. Chromium threw "Illegal invocation", so the ⌃G, C chord never ran. Node allows the call, so unit tests passed.
- **Rule:** Wrap injected timer functions in closures that call them unbound. Unit-test injected platform functions with strict functions that assert `this` is undefined.
- **Why:** Web platform functions check their receiver; Node's do not.

### Build fixtures deterministically
- **Date:** 2026-10-10
- **Trigger:** A DOCX fixture stamped zip members with the current time; a test that compared a written source with a fresh fixture failed when the run crossed a clock second.
- **Rule:** Fixtures that tests rebuild and compare must not depend on time, randomness or the environment. Set fixed zip member times.
- **Why:** Slow or loaded runs cross time boundaries; the failure looks random.

### Keep external revisions during save recovery
- **Trigger:** Post-publication rollback overwrote another writer's revision.
- **Rule:** Check both the publication inode and output hash before rollback. On conflict, keep the external file and a named recovery copy.
- **Why:** Publication does not grant ownership of later revisions at the path.

### Retain backups per source path
- **Trigger:** Same-named sources in a shared backup folder pruned each other's versions.
- **Rule:** Put a stable canonical-path identity in new backup names and pruning patterns. Leave ambiguous older names untouched.
- **Why:** A basename does not identify a document.

### Count non-text content in merged cells
- **Trigger:** An image-only merge continuation was silently discarded.
- **Rule:** Check mapped images, tables, breaks and text before treating a continuation as empty. Keep its content and report nonemptyMergeContinuation.
- **Why:** Empty paragraph text does not imply an empty cell.

### Bound actual archive inflation
- **Trigger:** Forged ZIP size fields hid expansion beyond the parser budget.
- **Rule:** Inflate in bounded chunks, count actual bytes against part and total limits, and check stream completion, length and CRC.
- **Why:** ZIP headers are untrusted; ZipExtFile can truncate oversized inflation to the declared length.

### Enforce XML policy at every parser
- **Trigger:** UTF-16 bytes hid DTD declarations from text regex checks.
- **Rule:** Reject alternate encodings and NUL bytes; use expat declaration handlers on all XML entry points, including unsupported parts and token parsing.
- **Why:** Validating one decoded form while parsing different bytes can bypass the policy.

### Bind review outcomes to proposal bytes
- **Trigger:** An old outcome marked a replaced proposal as accepted.
- **Rule:** Require the current proposal byte SHA-256 and an exact, disjoint accepted/declined node partition. Treat unbound outcomes as needsReview.
- **Why:** A filename and well-typed JSON do not bind a decision to the reviewed content.

### Index styles by immutable values
- **Trigger:** Many distinct styles made conversion take quadratic time.
- **Rule:** Keep ordered output lists and dictionary indexes keyed by immutable style, font, fill and numbering values.
- **Why:** Membership and index scans repeat work for each distinct item.

### Make a requested scan wait for a current result
- **Date:** 2026-10-10
- **Trigger:** `scan()` returned at once while a watcher-started scan ran. Switching Agent link on then returned an empty inbox, and the older scan's rows were dropped as a previous generation. The desktop suite failed or hung in about half of the runs.
- **Rule:** A scan requested during a running scan waits for it and then one more pass. Close watchers in test teardown even when an assertion fails.
- **Why:** Callers treat `await scan()` as "up to date"; a skipped request breaks that silently.

### Find the element that overflows before changing fit math
- **Date:** 2026-10-10
- **Trigger:** After the panel-fit fix, a 6-page public report still scrolled sideways. The zoom was correct; one rhwp text-box click target was 567 px wide and extended past the page.
- **Rule:** When a container scrolls, list the descendants whose right edge passes its client width before you change layout constants. Clip per-page click targets to the page.
- **Why:** rhwp can report text boxes wider than the page; a fit formula cannot compensate for content outside the page.

### Check disk space before fetching large dependencies
- **Date:** 2026-10-10
- **Trigger:** A shallow clone of rhwp (about 9 GB with samples and fonts), Cargo's git checkout and `cargo vendor` filled the disk; tool output files could not be written.
- **Rule:** Check free space and the upstream repository size first. Vendor only the library files the build needs, delete inspection copies when done, and share one Cargo target folder across worktrees.
- **Why:** Git dependencies vendor the whole package folder, and each worktree's debug build of rhwp and Tauri takes gigabytes.

### Check the screen before native window screenshots
- **Date:** 2026-10-10
- **Trigger:** The first Tauri window captured as an empty dark frame in five attempts. The page had loaded and sent IPC calls; the screen was locked, and earlier a leftover Electron test window covered it. WebKit does not paint views that are not visible.
- **Rule:** Before native visual QA, check `CGSessionCopyCurrentDictionary()` for a locked screen and list the windows in front. Use headless WebKit for visual checks while the user is away. Stop every test app you start, and confirm that it exited (`kill -0`); an unsaved-changes prompt can block a normal quit.
- **Why:** Playwright's Electron screenshots render without the screen; native window captures do not.

### Refuse table layouts that the writer cannot represent
- **Date:** 2026-10-10
- **Trigger:** The R6 port found that the Python DOCX writer keeps only one of two model cells that occupy the same grid position, with no loss report. Python integers also let an absurd border width pass, and one 1 MiB text run of tabs produced a million XML objects.
- **Rule:** When cells overlap, refuse the conversion with an error; never keep one and drop the other. Check generated control counts against the node budget before you allocate XML. Range-check numeric conversions and report `LIMIT` instead of saturating.
- **Why:** Writers that resolve conflicts quietly lose content that the loss report never mentions.

### Test saves on a real FAT or exFAT volume
- **Date:** 2026-10-10
- **Trigger:** Save in place failed on exFAT in both engines: hard links are unsupported (errno 45), the exchange rename fails with `ENOTSUP` (45, not `EOPNOTSUPP` 102 on macOS), and the copy fallback then refused because exFAT invents a new inode number per lookup.
- **Rule:** Try hard links and exchange first; on unsupported errors use verified exclusive copies and checked renames. Choose identity checks per volume (`fstatfs`): inodes where stable, else device, size, modification time and a descriptor-read SHA-256. Run the save probe on a mounted exFAT disk image (`hdiutil create -fs ExFAT`) as well as APFS; fault seams alone missed two of the three causes.
- **Why:** USB drives shared with Windows are usually exFAT; documents opened from them must save.

### Close stdin when you start Codex workers in the background
- **Date:** 2026-10-10
- **Trigger:** A background `codex exec ... "<prompt>"` worker ran for 6½ hours with 0.09 s of CPU time, no child processes and no session log. With a prompt argument, `codex exec` also reads a piped stdin to the end and appends it; the background shell never closed stdin, so the worker waited and did nothing.
- **Rule:** Start background workers with `</dev/null`. When a worker gives no output for more than a few minutes, check its CPU time, child processes and newest session log before you wait longer.
- **Why:** A worker that is waiting looks the same as a worker that is busy until you measure it.

### Start worker worktrees from the integration branch head
- **Date:** 2026-10-10
- **Trigger:** The R9a and R9b worktrees were created from the WebKit branch's WIP commit, which predated the R2, R6 and R7 merges. R9a found only stubs and stopped; the orchestrator had to stop it, merge `feat/rust` and restart it.
- **Rule:** Create each worker worktree from the head of the integration branch. When a worker also needs unmerged work from another branch, merge that branch into the integration branch first, or merge it into the new worktree before the worker starts. Before launching, run `git log --oneline <base>..<integration>`; it must be empty.
- **Why:** A worker cannot merge branches itself, so a stale base costs a full restart.

### Qualify renderer APIs and startup in WebKit
- **Date:** 2026-10-10
- **Trigger:** The WebKit control-mark test timed out after preview fell back to structure view; review Tab navigation skipped buttons; Rust file-open handoffs stayed queued.
- **Rule:** Use a cancellable timer fallback when idle callbacks are unavailable. Give review buttons explicit tab stops. Send the initial handoff-ready signal after event listeners are installed, and wait for subscription registration before releasing queued opens. Preserve the browser assertions and test both engines.
- **Why:** WebKit lacks idle callbacks and skips implicit button tab stops in its default keyboard mode. The Rust host requires a renderer readiness signal that Electron's load event previously supplied.
