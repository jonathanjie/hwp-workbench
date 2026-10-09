Apple Silicon Mac installation — bundled Python and Node; no Homebrew required.

# HWP Workbench 설치

1. DMG를 열고 **HWP Workbench**를 **Applications(응용 프로그램)**로 드래그합니다.
   macOS 13 이상이 필요합니다.
2. 응용 프로그램 폴더에서 앱을 엽니다. 이 알파 앱은 공증되지 않았습니다.
   macOS가 실행을 차단하면 **시스템 설정 → 개인정보 보호 및 보안 → 그래도 열기
   (Open Anyway)**를 선택합니다. 본인의 Claude Code에 아래 명령을 실행하도록
   요청하는 방법도 있습니다.

   ```bash
   xattr -dr com.apple.quarantine "/Applications/HWP Workbench.app"
   ```

3. F 키를 쓰려면 **시스템 설정 → 키보드 → 키보드 단축키 → 기능 키**에서
   **F1, F2 등의 키를 표준 기능 키로 사용**을 켭니다. 설정하지 않으면 `fn` 키와
   함께 누릅니다.
4. 한글 문서와 더 가까운 글꼴 모양이 필요하면 한컴의 공식 배포처에서 무료
   **함초롬바탕·함초롬돋움**을 받아 서체 관리자에 설치합니다. 앱에는 글꼴이
   포함되지 않습니다. 글꼴 설치만으로 동일한 페이지 배치를 보장하지는 않습니다.
5. 앱의 **Agents** 카드에서 Claude Code 또는 Codex를 선택합니다. 문서가 있는
   폴더를 지정하고 표시된 설정 명령을 본인의 터미널이나 에이전트에서 실행합니다.
   Python, Node.js 또는 Homebrew를 따로 설치할 필요가 없습니다. Claude Code와
   Codex 클라이언트는 각각 설치되어 있어야 합니다.

이 앱은 개발자 알파입니다. 공식 제출 전에는 저장한 파일을 Windows 한글에서 열어
내용과 배치를 확인합니다. 저장할 때마다 바뀌기 전의 파일은 문서 폴더의
`.hwp-workbench/backups/`에 보관됩니다(문서마다 최근 10개).
