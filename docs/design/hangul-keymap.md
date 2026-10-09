# Windows 한글 단축키 → Mac (HWP Workbench)

HWP Workbench는 Windows 한글을 쓰던 분이 Mac에서 같은 손 습관으로 작업하도록 만듭니다. 아래 표는 Windows 한글 단축키를 기억에 의존해 정리한 **최선의 추정**입니다. 틀린 키는 `desktop/keymap.js`의 표 한 곳만 고치면 앱 전체(메뉴, 명령 팔레트, 단축키 표)에 반영됩니다. Claude Code나 Codex에게 "keymap.js에서 ○○ 키를 △△로 바꿔 줘"라고 요청하면 됩니다.

*English: a best-effort map of Windows Hangul shortcuts. The single source of truth is the table in `desktop/keymap.js`; change a key there and the menus, palette and shortcut sheet follow.*

## Mac에서의 원칙

- Windows의 **Ctrl** = Mac의 **⌃ control** 또는 **⌘ command** (둘 다 동작).
- Windows의 **Alt** = Mac의 **⌥ option**.
- **F1–F12**: Mac은 기본적으로 `fn`을 함께 눌러야 합니다. 시스템 설정 → 키보드 → 키보드 단축키 → 기능 키에서 "F1, F2 등의 키를 표준 기능 키로 사용"을 켜면 Windows처럼 바로 동작합니다.
- **한/영 전환**: Mac은 Caps Lock(설정 시) 또는 한국어 키보드의 한/영 키. 앱이 이 키를 쓰지 않습니다.
- **한자 변환**: Mac 한국어 입력기의 ⌥ + Return (macOS 기능). 실제 입력 테스트로 확인 예정.
- macOS 시스템 키(⌃ Space 입력 소스, ⌘ Q, ⌘ W, ⌘ H, ⌘ Tab)는 덮어쓰지 않습니다.

## 저장

- **저장 (⌘S, ⌃S, ⌥S)**: 연 파일을 덮어씁니다. 덮어쓰기 전에 새 파일을 검증하고, 바꾸기 전의 파일은 앱 데이터 폴더에 보관합니다(문서마다 최근 10개). 다른 사람이 그사이 파일을 바꿨으면 먼저 묻습니다.
- **다른 이름으로 저장 (⌥V, ⇧⌘S)**: 새 파일로 저장합니다.

## 표

상태: **지금** = 지금 동작 · **C4** = 다음 데스크톱 작업에서 추가 · **엔진** = 서식 편집 기능이 먼저 필요 · **나중** = 아직 계획 없음

| 기능 | Windows 한글 | Mac (HWP Workbench) | 상태 |
|---|---|---|---|
| 불러오기 | Alt+O, Ctrl+O | ⌥O, ⌘O, ⌃O | 지금 (⌘O) · C4 (나머지) |
| 저장 (덮어쓰기, 이전 버전 보관) | Alt+S, Ctrl+S | ⌥S, ⌘S, ⌃S | C4 |
| 다른 이름으로 저장 | Alt+V | ⌥V, ⇧⌘S | 지금 (⌘S, 사본) · C4 |
| 새 문서 | Alt+N | ⌥N | 나중 |
| 인쇄 | Alt+P | ⌥P, ⌘P | 나중 |
| 되돌리기 | Ctrl+Z | ⌘Z, ⌃Z | 지금 (⌘Z) · C4 |
| 다시 실행 | Ctrl+Shift+Z | ⇧⌘Z, ⇧⌃Z | 지금 (⇧⌘Z) · C4 |
| 모두 선택 | Ctrl+A | ⌘A, ⌃A | 지금 (⌘A) · C4 |
| 찾기 | Ctrl+F | ⌘F, ⌃F | 지금 (⌘F) · C4 |
| 찾아 바꾸기 | Ctrl+H (또는 Ctrl+F2?) | ⌃H, ⌥⌘F | 지금 (⌥⌘F) · C4 |
| 글자 모양 | Alt+L | ⌥L | C4 (보기만) · 엔진 (변경) |
| 문단 모양 | Alt+T | ⌥T | C4 (보기만) · 엔진 (변경) |
| 편집 용지 | F7 | F7 | C4 (보기만) |
| 스타일 | F6 | F6 | C4 (보기만) |
| 문자표 | Ctrl+F10 | ⌃F10, ⌘F10 | C4 |
| 조판 부호 보이기 | Ctrl+G, C | ⌃G 다음 C | C4 |
| 셀 블록 | F5 | F5 | C4 (셀 이동·편집) |
| 다음 / 이전 셀 | Tab / Shift+Tab | Tab / ⇧Tab | C4 |
| 한자 변환 | F9, 한자 키 | ⌥ Return (macOS) | 확인 예정 |
| 표 만들기 | Ctrl+N, T | ⌃N 다음 T | 엔진 |
| 쪽 나누기 | Ctrl+Enter | ⌃Return | 엔진 |
| 진하게 / 기울임 / 밑줄 | Ctrl+B / I / U | ⌘B / I / U | 엔진 |
| 정렬 (왼쪽/가운데/오른쪽/양쪽) | Ctrl+Shift+L / C / R / M (?) | ⇧⌃L / C / R / M | 엔진 |
| 글자 크게 / 작게 | Alt+Shift+E / R (?) | ⇧⌥E / R | 엔진 |
| 자간 좁게 / 넓게 | Alt+Shift+N / W (?) | ⇧⌥N / W | 엔진 |
| 장평 좁게 / 넓게 | Alt+Shift+J / K (?) | ⇧⌥J / K | 엔진 |
| 모양 복사 | Alt+C | ⌥C | 엔진 |

(?) 표시는 특히 확실하지 않은 키입니다.

## 고치는 방법

키가 다르면 `desktop/keymap.js`의 표를 고치고 이 문서의 같은 줄도 고칩니다. `npm run test:desktop`이 표의 형식과 겹치는 키를 검사합니다.
