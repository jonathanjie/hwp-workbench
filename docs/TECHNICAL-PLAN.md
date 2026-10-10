# HWP Workbench — technical implementation plan

Version 0.2.0-alpha.2 · 10 October 2026 · Developer alpha · Apple Silicon macOS first

## 1. Product contract

The required product is a Korean/English desktop word processor that can fully edit HWP and HWPX while closely preserving official-document formatting. It must expose the same document operations to human users and agents, support source development in the public repository, and start validation with publicly obtainable documents. These requirements have not been relaxed to text extraction or an HWP-to-DOCX converter.

The initial build is a narrower, working engineering foundation. It implements conservative text edits, source preservation, explicit compatibility warnings, reviewable patches and shared desktop/CLI/MCP operations. It does not yet meet the full editing or visual-fidelity requirement. “Implemented,” “tested,” and “required for release” are tracked separately throughout this plan.

### Acceptance definition for the finished product

A supported official document must open without repair warnings in the reference Hancom installation after editing. Intended text, formatting, structure, links and object changes must be present; untouched content must remain intact. The app must identify unsupported features before a potentially destructive save. For supported no-op and non-reflowing edits, page count, pagination, table geometry and anchored objects must match the reference within an explicitly agreed tolerance. Text insertion that legitimately changes layout is compared with the same edit made in Hancom, not with the unedited original.

“Fully editable” covers ordinary unprotected HWP 5.x and HWPX authoring: paragraphs, character/paragraph styles, sections, page setup, numbering, tables including merged/nested cells, images, stamps, headers/footers, fields, bookmarks and annotations. Equations, charts, OLE and revision semantics need individual support decisions and tests rather than a blanket compatibility promise. HWP 3.x and password/DRM-protected workflows require separate scope decisions. The app will not bypass protection.

## 2. Architecture and decisions

### Current, delivered architecture

```text
Tauri WebKit window: Korean/English renderer
             │ named, validated invoke commands
             ▼
app-core: native dialogs, granted paths, document session and drafts
             │ shared in-process Rust service
             ▼
Rust preservation engine + vendored rhwp verifier/exporter
             ▲                              ▲
       headless CLI                    stdio MCP server
             └──── explicitly allowed local workspace ────┘

Source bytes → parsed text/structure + opaque parts → complete validated patch
             → native same-format writer → self-reopen and rhwp checks
             → exclusive new output, or verified atomic save with backup
```

Rust is the canonical engine. The same operation contract serves CLI, MCP and
desktop. The Tauri shell is thin; session, drafts, agent inbox and path checks
live in `crates/app-core`. The app executable also runs CLI and MCP commands.
The modular HTML/CSS/JavaScript renderer uses rhwp WASM for native page display
and receives only named operations through `window.hwp`. It never owns native
serialization. The independent native reader runs in a bounded worker process.

### Production representation

Maintain three related representations: original native payloads and provenance; an editable semantic document tree; and a computed layout tree with pages, lines, glyph runs and object geometry. Every semantic node must retain a mapping to source-native records or XML. Unrecognized data stays attached to its native location with explicit support status.

Do not use HTML, Markdown or a generic rich-text editor's internal state as the authoritative save format. They can serve as display or editing adapters, but a native operation must carry each change back to HWP/HWPX. Saving an HTML reconstruction would erase information that the browser never represented. HWPX uses an XML-based package, whereas legacy HWP uses a binary structure; native adapters are necessary for both [S1].

The Rust core, CLI and rhwp renderer are the delivered architecture. React and a mature rich-text toolkit remain optional UI components; selection, IME, history and clipboard support must not require flattening the native document model.

### Compatibility references and alternatives

| Candidate | Evaluate for | Required evidence before adoption |
|---|---|---|
| `STAIxBWLB/hwp-cli` | Native HWP/HWPX operations, writer, rendering and a second parser | Corpus edit/reopen fidelity, unknown-object retention, bounded malformed-input handling, dependency/license review |
| `edwardkim/rhwp` | Rust/WASM layout and interactive editing | Mac font shaping, native-format save behavior, table/section fidelity, interoperability on the same fixture corpus |
| `muin-company/handoc` | HWPX-specific parsing/UI integration | Actual legacy-HWP writing coverage and whether its model preserves opaque native data |
| Official Hancom HWP SDK | A licensed compatibility path/reference | Written confirmation of Apple Silicon/macOS capabilities, redistribution terms, offline behavior and cost before procurement |

The hwp-cli project itself distinguishes lossless no-op behavior from edited synthesis and explicitly does not claim Hancom parity [S2]. rhwp documents a Rust/WASM HWP viewer/editor [S3]; HanDoc advertises HWPX tooling but is not independently qualified for our complete HWP-writing requirement [S4]. rhwp 0.8.7 is vendored and bundled under its MIT licence. No commercial SDK has been licensed or integrated. Compare alternatives on the same corpus; README feature counts do not establish compatibility.

## 3. Initial implementation and evidence

| Area | Implemented now | Evidence and remaining boundary |
|---|---|---|
| HWPX reading/writing | Bounded ZIP/XML parsing; namespace-aware text addresses; surgical changes to existing plain text; original-format save-as | Variable-length text and table-cell edits exercised on synthetic fixtures; full XML schema and Hancom behavior unverified |
| HWP reading/writing | CFB v3/v4, mini-streams, FAT/DIFAT; HWP5 records; variable-length edits inside existing plain-text runs | Authored preservation tests; native controls retained; Hancom reflow unverified |
| Preservation | Exact no-op bytes; untouched payload retention; complete hash/expected-text validation; native self-reparse and rhwp checks | Independent text-presence evidence does not certify layout or Hancom acceptance |
| Desktop | Tauri shell, bilingual renderer, staged history/review, native save with backup, save-copy and agent links | WebKit workflows exercise renderer/backend; native QA and Hancom remain separate gates |
| Agent interface | CLI plus 13 stdio MCP tools; explicit workspace; structured diffs/errors | Core tests include subprocess CLI/MCP; compatibility profile is explicitly versioned |
| Distribution | Rust/Tauri Mac package builder, generated dependency notices and one macOS arm64 CI job | Developer alpha; CI execution and signed/notarized release qualification require separate evidence |

Current check commands and evidence boundaries are in `docs/TESTING.md`. The rewrite plan records earlier gate evidence. Keep fresh reports under `outputs/` with host and platform information; do not carry forward a historical result as a new pass.

The synthetic fixtures are deliberately controlled parser/rewriter test inputs. Some opaque contents are placeholders, not complete native objects. They do not establish that a Hancom installation accepts any generated document. Passing round trips with our own reader and writer is necessary, but circular as an interoperability proof.

## 4. Preservation policy

HWPX changes replace only selected text spans inside the original XML bytes. Unchanged ZIP members keep their uncompressed bytes; member order and metadata are carried forward where supported. Edited archives are not promised to be byte-identical ZIP containers. Existing preview and line-layout caches remain unchanged and may be stale. This alpha does not calculate a new page layout or maintain every text-relative attribute following a length-changing edit; those are explicit release blockers.

HWP changes allow different UTF-16 code-unit lengths within existing plain-text runs, with paragraph lengths, style starts, range tags and cached line starts updated. Native controls remain untouched. The writer rebuilds compound-file allocation structures while retaining stream payloads not affected by the edit, then verifies the stream inventory. Any allowed text edit can change glyph width and layout; Hancom reflow remains unverified.

The `apply` and conversion paths never overwrite. Explicit `save` verifies the complete native result, keeps a private previous version, refuses disk conflicts and atomically replaces the opened file. Every edit batch is validated completely before mutation; its source SHA-256 and expected node text must match. A new output is written privately and atomically published only if the destination does not already exist. Unsupported/protected cases fail closed where detected. The current feature detector is not exhaustive, so a general layout-unverified warning remains mandatory.

Protected documents and active content are not executed. Script storage makes a document read-only except for the exact hash-qualified inert Hancom default stub; even that stub remains byte-identical and is never executed. Improve classification using actual fixtures; never quietly remove script/signature streams to make tests pass.

## 5. Public corpus and the independent reference

Three source fixtures were located and recorded with immutable Git blob IDs and expected sizes: pyhwp alignment and border/fill HWP samples, plus an hwp-cli HWPX report with tables. The local downloader verifies blob identity and size and records SHA-256. These sources are engineering fixtures, not yet a representative government-form corpus. The Node downloader never replaces existing files. Rust corpus tests use files when present and report missing files as skipped. No public-file result is inferred from authored samples.

Next, expand to at least 40 initially selected documents spanning public Korean forms, reports, procurement applications and correspondence, together with feature-focused synthetic cases. This is a proposed test target, not an achieved count. Log source URL, retrieval date, allowed test use, checksum, format/version, fonts and feature inventory. Keep confidential user examples outside source control. Public availability is not treated as unrestricted redistribution permission.

For each accepted fixture, capture an original reference from a licensed Hancom environment with a fixed version, OS, installed fonts, page size and print settings. Record page count, text content/order, table topology, cell geometry, object inventory/anchors and reference images. Then perform no-op save, one targeted edit, multi-edit batches, repeated save/reopen, and deliberately unsupported operations. Compare to the equivalent edit performed in Hancom.

Use separate results for binary preservation, native structural validity, independent reopen, semantic differences and visual differences. No-op files should be byte-identical on the preservation path. Unchanged assets should have identical hashes. For visual checks, establish tolerances using the reference environment's own repeatability; a single aggregate pixel-similarity score can hide missing seals or a shifted signature field. Critical official-form regions require semantic/geometry checks plus visual review.

## 6. Implementation milestones and exit gates

### M0 — Delivered foundation

Current source, CLI/MCP, conservative writers, renderer workflow, authored tests, a Rust/Tauri package builder and CI. Historical gate evidence is in the rewrite plan. This milestone does not authorize use for submissions.

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

The Rust engine and rhwp are bundled in the Apple Silicon Tauri app. Qualify clean-machine installation and native behaviour. Pin dependencies and review licenses, generate an SBOM, add native crash recovery and migration tests, and perform parser fuzzing/security review. Implement a macOS-specific least-privilege worker boundary for the native engine and worker.

Use an authorized Apple Developer identity for Developer ID signing and notarization, secure release secrets outside the repository, and exercise clean-machine installation. Publish source and release artifacts only within the approved scope; exclude credentials and user documents. The current ad-hoc packaging script is a developer convenience, not this release process.

Exit: clean Apple Silicon install/start/edit/save/reopen tests; valid signing/notarization results; passing real-corpus gates; explicit support matrix; reproducible release artifacts; an internal user acceptance session. macOS packaging can progress in parallel, but release cannot bypass document-fidelity gates.

## 7. Security, privacy and operational design

The renderer receives named Tauri commands and has a restrictive content security
policy. It displays document text through text nodes, with no source-document
HTML execution, generic shell command or filesystem API. Native dialogs and
backend grants authorize paths. External navigation and requests are restricted.

The Rust engine bounds input and expansion sizes, rejects DTD/entity declarations
and unsafe archive paths, and never executes document code. Workspace access
uses directory descriptors and no-follow opens. Native engine code and its
verification worker are not OS-sandboxed; input validation and worker timeout
are not a complete process security boundary. See `docs/SECURITY.md`.

CLI/MCP roots are chosen by the process owner. An MCP acknowledgement is not human consent enforcement; configure the agent host's permissions and avoid broad roots such as the whole home directory. Document content is untrusted data, including any text claiming to instruct an agent. The workbench itself does not upload documents, but an external agent/model host may transmit tool results under its own privacy settings.

Staged draft files can contain confidential text and are stored locally without application-layer encryption. Before release, add user-visible retention/deletion controls and consider platform-protected storage. Source preservation is not a replacement for a user backup policy. Review Git staging before publication; generated artifacts, patches and screenshots can also contain sensitive content.

## 8. Handoff and next decision

The source package contains the canonical Rust engine, CLI/MCP, thin Tauri shell,
renderer, frozen authored fixtures, a download-only public-fixture tool and
Rust/Node CI. Cut-over is recorded in the rewrite plan. Real public-file and
Hancom qualification, font metrics and signed/notarized distribution remain
release gates. Native QA requires a quiet minute and separate authorization
when GUI launch is prohibited. No official-file visual match is claimed.

## Sources consulted

[S1] Hancom, HWPX package structure: https://tech.hancom.com/hwpxformat/

[S2] hwp-cli implementation status and limitations: https://github.com/STAIxBWLB/hwp-cli

[S3] rhwp Rust/WASM viewer/editor: https://github.com/edwardkim/rhwp

[S4] HanDoc project: https://github.com/muin-company/handoc


[S6] Hancom HWP SDK product information: https://www.hancom.com/product/sdk/hwpSdk

[S7] Public fixture provenance and fixed blob IDs: `fixtures/public-manifest.json`; linked upstream repositories remain the sources of truth for fixture terms and history.

[S8] Current checks: `docs/TESTING.md`; historical gate results: `docs/plans/2026-10-10-rust-tauri-rewrite.md`.
