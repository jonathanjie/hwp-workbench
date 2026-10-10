# Inline space markers

Goal: edit native HWPX text around simple inline markers without losing native bytes.

Design: the shared Rust document model assigns a different U+FDD0..U+FDEF noncharacter to each childless `fwSpace`, `nbSpace`, `tab`, `lineBreak` or `hyphen` in a text node. It retains the original element byte range, including attributes and paired empty syntax. Nodes with other children, placeholder collisions or more than 32 markers stay locked. A patch can keep, move or delete original markers, but cannot duplicate or invent one. Placeholders are local to a source revision and are assigned again after reopen.

The native writer escapes ordinary text and restores retained marker bytes. Verification compares edited XML bytes and projected reader text. Independent rhwp checks receive reader text rather than placeholders and fall back to exact native nested paragraph text when Unicode export omits a list. `inspect` exposes the same reader map as `inlineMarkers`; desktop hit mapping, search marks and preview offsets use it. Explicit conversion text checks also project markers. Binary HWP retains its current plain-run boundary: space controls are not editable and their bytes remain unchanged.

The slip keeps a native textarea for caret and Korean composition. A display mirror outlines each placeholder with the same font metrics. It does not change textarea text, selection or IME events. Validation refuses a placeholder with no original marker.

Execution:
1. Add Rust fixture-injection tests for native retention, deletion, collisions and refusals. Run and record the expected failures.
2. Add marker parsing, validation, surgical serialization and verification in `hwp-core`; adapt independent checks in the shared service. Run targeted tests.
3. Add Node slip and edit-model tests; implement visible marker handling without changing IME event logic.
4. Document marker and binary HWP contracts. Run all brief gates on this host, with no GUI or external fixture downloads.
5. Test the private form locally, save the consent edit in place with a retained backup, and record counts without copying document content into the repository.

No git writes. No Python or Electron changes. Final evidence: `work/results/C2.json`.
