# Real Korean IME test on macOS

- **Date:** 2026-10-09
- **Host:** macOS 26 (Darwin 25.6.0), Apple Silicon (arm64), Electron 44.5.1
- **Input source:** 2-Set Korean (`com.apple.inputmethod.Korean.2SetKorean`)
- **Method:** `scripts/qa/real-ime.mjs` posts real key events through the macOS HID event tap (`scripts/qa/ime-keys.swift`), so the system input method composes the text. The page logs every `keydown`, `composition*` and `beforeinput` event. A bare textarea in the same window is the control.

## Results

| Case | Keys (2-Set) | Control | Proof slip before fix | Proof slip after fix |
|---|---|---|---|---|
| Type 한글 (×3, plus 1 earlier run) | g k s r m f | 한글 | 한글 | 한글 |
| Final consonant moves on: 하나 | g k s k | 하나 | 하나 | not rerun with real keys |
| Jamo Backspace | g k s k ⌫ | 하ㄴ | 하ㄴ | not rerun with real keys |
| Mixed Korean and English | g k s, then ABC a b | — | 한ab | not rerun with real keys |
| Enter after 글 | … f ↵ | 한글 + line break | 한글 + line break (defect) | Replay test: no line break |
| ⌘↵ while 글 composes | … f ⌘↵ | — | Nothing staged; a second ⌘↵ staged 한글 once (defect) | Replay test: stages 한글 once |

No duplicated syllable occurred in any clean run.

## Defects found and fixed

1. **Enter added a line break.** macOS sends Enter twice: first with `isComposing` set, which commits the syllable, then as a plain Enter. The slip let the plain Enter insert a line break. Paragraph text cannot contain one. The slip now blocks a plain Enter.
2. **⌘↵ during composition was lost.** macOS sends ⌘↵ once, with `isComposing` set, and sends no second key. The slip ignored it. The slip now stages after `compositionend`, so the committed syllable is staged exactly once.
3. **Escape during composition closed the slip.** The slip now closes only on an Escape that does not belong to a composition.

`scripts/test-electron.mjs` replays the recorded key sequence. It fails on the old slip and passes on the fixed slip.

## Not tested

- Real-key runs of the fixed slip for recomposition, Backspace and mixed input. Run `scripts/qa/real-ime.mjs` while nobody types.
- Composition cancel with Escape using real keys, selection replacement during composition, decomposed Hangul (NFD), emoji, and the 3-Set Korean layouts.
- Earlier runs lost keys because a person was typing in another app at the same time. Those runs are not counted.
