# HWP Workbench — technical implementation plan

Version 0.1.0-alpha.1 · 9 October 2026 · Internal/private · Apple Silicon macOS first

## 1. Product contract

The required product is a Korean/English desktop word processor that can fully edit HWP and HWPX while closely preserving official-document formatting. It must expose the same document operations to human users and agents, support private GitHub development, and start validation with publicly obtainable documents. These requirements have not been relaxed to text extraction or an HWP-to-DOCX converter.

The initial build is a narrower, working engineering foundation. It implements conservative text edits, source preservation, explicit compatibility warnings, reviewable patches and shared desktop/CLI/MCP operations. It does not yet meet the full editing or visual-fidelity requirement. “Implemented,” “tested,” and “required for release” are tracked separately throughout this plan.

### Acceptance definition for the finished product

A supported official document must open without repair warnings in the reference Hancom installation after editing. Intended text, formatting, structure, links and object changes must be present; untouched content must remain intact. The app must identify unsupported features before a potentially destructive save. For supported no-op and non-reflowing edits, page count, pagination, table geometry and anchored objects must match the reference within an explicitly agreed tolerance. Text insertion that legitimately changes layout is compared with the same edit made in Hancom, not with the unedited original.

“Fully editable” covers ordinary unprotected HWP 5.x and HWPX authoring: paragraphs, character/paragraph styles, sections, page setup, numbering, tables including merged/nested cells, images, stamps, headers/footers, fields, bookmarks and annotations. Equations, charts, OLE and revision semantics need individual support decisions and tests rather than a blanket compatibility promise. HWP 3.x and password/DRM-protected workflows require separate scope decisions. The app will not bypass protection.

## 2. Architecture and decisions

### Current, delivered architecture

```text
Electron window: Korean/English renderer
             │ narrow, validated preload IPC
             ▼
Electron main: native dialogs, granted paths, patch recovery
             │ bounded JSON subprocess request; no shell
             ▼
Python preservation engine ── shared invoke() operation layer
             ▲                              ▲
       headless CLI                    stdio MCP server
             │                              │
             └──── explicitly allowed local workspace ────┘

Source bytes → parsed text/structure + opaque parts → validated patch
             → native same-format writer → self-reopen checks
             → atomic creation of a new output file
```

The alpha uses plain modular JavaScript/HTML/CSS for the Electron renderer and a standard-library Python engine. It does not contain React, ProseMirror, Rust, a vendored HWP library, or a hidden conversion service. This differs from the earlier proposed stack intentionally: the first executable deliverable is a small inspectable preservation path that can be tested without external dependencies. Network restrictions prevented installing or independently exercising the candidate Rust engines in this environment.

The important durable boundary is the operation contract, not the first language choice. A future native engine can implement `inspect`, `plan`, `apply`, rendering and structural operations behind the same versioned boundary. The GUI must never become the only place document manipulation is possible.

### Production representation

Maintain three related representations: original native payloads and provenance; an editable semantic document tree; and a computed layout tree with pages, lines, glyph runs and object geometry. Every semantic node must retain a mapping to source-native records or XML. Unrecognized data stays attached to its native location with explicit support status.

Do not use HTML, Markdown or a generic rich-text editor's internal state as the authoritative save format. They can serve as display or editing adapters, but a native operation must carry each change back to HWP/HWPX. Saving an HTML reconstruction would erase information that the browser never represented. HWPX uses an XML-based package, whereas legacy HWP uses a binary structure; native adapters are necessary for both [S1].

A Rust core with a native CLI and a WASM/native renderer remains the preferred production candidate after evaluation. React and a mature rich-text toolkit remain optional UI components; selection, IME, history and clipboard support must not require flattening the native document model.

### Candidate-engine evaluation, not a dependency commitment

| Candidate | Evaluate for | Required evidence before adoption |
|---|---|---|
| `STAIxBWLB/hwp-cli` | Native HWP/HWPX operations, writer, rendering and a second parser | Corpus edit/reopen fidelity, unknown-object retention, bounded malformed-input handling, dependency/license review |
| `edwardkim/rhwp` | Rust/WASM layout and interactive editing | Mac font shaping, native-format save behavior, table/section fidelity, interoperability on the same fixture corpus |
| `muin-company/handoc` | HWPX-specific parsing/UI integration | Actual legacy-HWP writing coverage and whether its model preserves opaque native data |
| Official Hancom HWP SDK | A licensed compatibility path/reference | Written confirmation of Apple Silicon/macOS capabilities, redistribution terms, offline behavior and cost before procurement |

The hwp-cli project itself distinguishes lossless no-op behavior from edited synthesis and explicitly does not claim Hancom parity [S2]. rhwp documents a Rust/WASM HWP viewer/editor [S3]; HanDoc advertises HWPX tooling but is not independently qualified for our complete HWP-writing requirement [S4]. No third-party engine is bundled, and no commercial SDK has been licensed or integrated. Run all candidates through the same test protocol rather than comparing README feature counts.

## 3. Initial implementation and evidence

| Area | Implemented now | Evidence and remaining boundary |
|---|---|---|
| HWPX reading/writing | Bounded ZIP/XML parsing; namespace-aware text addresses; surgical changes to existing plain text; original-format save-as | Variable-length text and table-cell edits exercised on synthetic fixtures; full XML schema and Hancom behavior unverified |
| HWP reading/writing | CFB v3/v4 traversal, mini-streams, FAT/DIFAT; compressed/uncompressed HWP5 paragraph text; equal-UTF-16-length replacement | Synthetic extended-record, allocation-table and preservation tests pass; no general text-length changes or native layout |
| Preservation | No-op exact byte return; unchanged payload retention; source-hash/expected-text checks; post-write self-reparse | Self-consistency only, not an independent reader or Hancom acceptance |
| Desktop | Bilingual shell, outline search, text selection/inspector, staged history, patch export, review dialog, staged draft recovery and save-copy wiring | 8 native Electron workflows pass on macOS arm64; real 2-Set Korean IME tested with OS key events (`reports/ime-macos.md`); packaging and signing not run |
| Agent interface | CLI plus five stdio MCP tools; explicit workspace; structured diffs/errors | Core tests include subprocess CLI/MCP; compatibility profile is explicitly versioned |
| Distribution | Private repo script, macOS bootstrap, developer app packaging, core CI workflow | Provided as source; no remote repository, CI execution, notarization or Mac binary produced |

The delivered report records **68 passing core/CLI/MCP tests on Linux/Python 3.13.5**, with no skips or failures. The renderer report records **15 checks and no browser errors**. The renderer was loaded from local source into an in-memory Chromium page because browser navigation was restricted; its filesystem bridge calls the real engine. This validates renderer behavior, not Electron security configuration, native file loading or a packaged Mac application.

The synthetic fixtures are deliberately controlled parser/rewriter test inputs. Some opaque contents are placeholders, not complete native objects. They do not establish that a Hancom installation accepts any generated document. Passing round trips with our own reader and writer is necessary, but circular as an interoperability proof.

## 4. Preservation policy

HWPX changes replace only selected text spans inside the original XML bytes. Unchanged ZIP members keep their uncompressed bytes; member order and metadata are carried forward where supported. Edited archives are not promised to be byte-identical ZIP containers. Existing preview and line-layout caches remain unchanged and may be stale. This alpha does not calculate a new page layout or maintain every text-relative attribute following a length-changing edit; those are explicit release blockers.

HWP changes currently require exactly the same UTF-16 code-unit length for each selected text segment. Native controls remain untouched. The writer rebuilds compound-file allocation structures while retaining stream payloads not affected by the edit, then verifies the stream inventory. Equal character count does not imply equal glyph width, so even allowed edits can visually differ until verified against Hancom.

The original source is never overwritten. Every edit batch is validated completely before mutation; its source SHA-256 and expected node text must match. A new output is written privately and atomically published only if the destination does not already exist. Unsupported/protected cases fail closed where detected. The current feature detector is not exhaustive, so a general layout-unverified warning remains mandatory.

Protected documents and active content are not executed. The conservative HWP policy marks documents with script storage read-only, even when the storage might merely be an inert template. This will reject editing in some otherwise ordinary real files. Improve classification using actual fixtures; never quietly remove script/signature streams to make tests pass.

## 5. Public corpus and the independent reference

Three source fixtures were located and recorded with immutable Git blob IDs and expected sizes: pyhwp alignment and border/fill HWP samples, plus an hwp-cli HWPX report with tables. The local downloader verifies blob identity and size and records SHA-256. These sources are engineering fixtures, not yet a representative government-form corpus. Downloads failed because external hostnames could not be resolved from the execution container; **their binary tests did not run**. Reports preserve that failure instead of counting missing files as success.

Next, expand to at least 40 initially selected documents spanning public Korean forms, reports, procurement applications and correspondence, together with feature-focused synthetic cases. This is a proposed test target, not an achieved count. Log source URL, retrieval date, allowed test use, checksum, format/version, fonts and feature inventory. Keep confidential user examples outside source control. Public availability is not treated as unrestricted redistribution permission.

For each accepted fixture, capture an original reference from a licensed Hancom environment with a fixed version, OS, installed fonts, page size and print settings. Record page count, text content/order, table topology, cell geometry, object inventory/anchors and reference images. Then perform no-op save, one targeted edit, multi-edit batches, repeated save/reopen, and deliberately unsupported operations. Compare to the equivalent edit performed in Hancom.

Use separate results for binary preservation, native structural validity, independent reopen, semantic differences and visual differences. No-op files should be byte-identical on the preservation path. Unchanged assets should have identical hashes. For visual checks, establish tolerances using the reference environment's own repeatability; a single aggregate pixel-similarity score can hide missing seals or a shifted signature field. Critical official-form regions require semantic/geometry checks plus visual review.

## 6. Implementation milestones and exit gates

### M0 — Delivered foundation

Current source, CLI/MCP, conservative writers, renderer workflow, synthetic tests and setup scripts. Exit evidence is in `reports/`. This milestone does not authorize use for submissions.

### M1 — Real-format and Mac qualification

Run the public corpus and native Mac launch first. Verify actual HWP5 and HWPX opening in Hancom before broadening the UI. Obtain published format specifications, review parser bounds and malformed-input handling, and test all engine candidates on identical data. Classify unsupported native features and font requirements. Establish separate read, preserve, edit, render and export coverage for each feature.

Exit: recorded real-file edit/save/reopen outcomes, a chosen/pinned engine strategy, a reproducible Apple Silicon development launch, and no undocumented silent drops on the qualified corpus. Failures remain visible and route to read-only mode where needed.

### M2 — General native text editing and layout

Implement variable-length HWP text operations with correct paragraph lengths, character-style offsets, range metadata, control relationships and line-layout invalidation/recomputation. Add paragraph/run insertion, deletion, split/join, styles, alignment, indents, tabs, numbering and section/page settings. Implement equivalent native HWPX updates rather than inserting arbitrary HTML. Couple edits to a real layout engine, native font discovery and explicit missing-font diagnostics.

Exit: Korean/English typing, deletion, selection, clipboard, undo/redo and pagination agree with reference edits across the qualified simple-document corpus. Test 2-beolsik Korean IME, composition cancellation, final-consonant recomposition, mixed scripts, decomposed/composed Hangul and emoji boundaries on actual macOS.

### M3 — Official-form editing

Add table row/column insertion/deletion, resizing, merged/nested cells, cell margins/borders/fills and page splitting; headers/footers; page/section breaks; fields and bookmarks; images/seals with anchors and wrapping. Preserve native identities and links on copy/paste. Provide field navigation, keyboard-first controls, clear selected-cell boundaries and localized diagnostics. Unsupported embedded elements remain visible and protected instead of disappearing.

Exit: all qualified official-form tests reopen without repair prompts; intended table/field/seal geometry matches the Hancom-edited references. Unsupported edits are blocked or explicitly routed to an approved fallback.

### M4 — Full workflow and agents

Add new-document authoring, templates, document properties, print/PDF export, broader comments/revision support and multi-document workflow. Add native export adapters individually; DOCX conversion is secondary to correct HWP/HWPX saves. Evaluate equations/charts/OLE with a documented feature matrix; do not claim complete editing from an opaque-preservation implementation.

Extend the shared operation contract for styles, table cells, fields and structures, with document-revision IDs, atomic transactions, dry-run reports, cancellation, batch limits and capability discovery. Add current MCP protocol support and client interoperability tests while retaining a documented compatibility profile. Every UI command must have an equivalent headless operation or an explicit UI-only classification.

Exit: supported GUI actions have deterministic agent tests; stale or ambiguous requests cannot edit the wrong document; clients can discover exactly which native operations are supported.

### M5 — Internal release qualification

Bundle the selected native engine/runtime into an Apple Silicon `.app`; remove the external Python prerequisite when appropriate. Pin dependencies and review licenses, generate an SBOM, add native crash recovery and migration tests, and perform parser fuzzing/security review. Implement a macOS-specific least-privilege worker boundary rather than relying only on the Electron renderer sandbox.

Use an authorized Apple Developer identity for Developer ID signing and notarization, secure release secrets outside the repository, and exercise clean-machine installation. Publish only to the approved private repository/release channel. The current ad-hoc packaging script is a developer convenience, not this release process.

Exit: clean Apple Silicon install/start/edit/save/reopen tests; valid signing/notarization results; passing real-corpus gates; explicit support matrix; reproducible release artifacts; an internal user acceptance session. macOS packaging can progress in parallel, but release cannot bypass document-fidelity gates.

## 7. Security, privacy and operational design

The delivered renderer has no Node integration, uses a narrow preload API and runs with context isolation and sandbox flags. It displays text through text nodes rather than executing source-document HTML. Permission requests, external navigation and network requests are denied by app wiring. These choices follow Electron's recommended separation of untrusted content and privileged APIs [S5], but that wiring has not yet been exercised in Electron here.

The engine bounds input and expansion sizes, rejects DTD/entity declarations and unsafe archive paths, and does not execute scripts or embedded programs. Filesystem access is restricted by explicit workspace roots, directory descriptors and no-follow opens. Python workers are not OS-sandboxed in this alpha; parser validation and a desktop subprocess timeout are not a complete security boundary.

CLI/MCP roots are chosen by the process owner. An MCP acknowledgement is not human consent enforcement; configure the agent host's permissions and avoid broad roots such as the whole home directory. Document content is untrusted data, including any text claiming to instruct an agent. The workbench itself does not upload documents, but an external agent/model host may transmit tool results under its own privacy settings.

Staged draft files can contain confidential text and are stored locally without application-layer encryption. Before release, add user-visible retention/deletion controls and consider platform-protected storage. Source preservation is not a replacement for a user backup policy. Review Git staging before publication; generated artifacts, patches and screenshots can also contain sensitive content.

## 8. Handoff and next decision

The source package includes engine, desktop sources, CLI/MCP entrypoints, test fixtures, actual test reports, screenshots, sample patches, a public-fixture downloader, a private publishing script, macOS bootstrap/packaging scripts and CI configuration. No GitHub repository was created, no macOS build was executed, and no official-file visual match is claimed.

No further product requirements are needed to start M1. The next engineering decision is the native engine choice, based on real-format and Hancom reference results. User sample documents, a licensed reference environment, an approved GitHub owner and signing access are inputs to later qualification, not reasons to replace the full requirement with a text-only product.

## Sources consulted

[S1] Hancom, HWPX package structure: https://tech.hancom.com/hwpxformat/

[S2] hwp-cli implementation status and limitations: https://github.com/STAIxBWLB/hwp-cli

[S3] rhwp Rust/WASM viewer/editor: https://github.com/edwardkim/rhwp

[S4] HanDoc project: https://github.com/muin-company/handoc

[S5] Electron security guidance: https://www.electronjs.org/docs/latest/tutorial/security

[S6] Hancom HWP SDK product information: https://www.hancom.com/product/sdk/hwpSdk

[S7] Public fixture provenance and fixed blob IDs: `fixtures/public-manifest.json`; linked upstream repositories remain the sources of truth for fixture terms and history.

[S8] Local execution evidence: `reports/test-results.json`, `reports/test-log.txt`, `reports/ui-test-results.json`, `reports/public-fixture-downloads.json`, `reports/public-corpus-results.json`.
