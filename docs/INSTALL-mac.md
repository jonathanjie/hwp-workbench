# macOS 설치 / Installation

## 한국어

Apple Silicon Mac용 개발자 알파입니다. macOS 12 이상이 필요합니다.
한컴의 저장 후 다시 열기와 문서 배치는 아직 검증되지 않았습니다.
공문서를 제출하기 전에 저장한 파일을 한글에서 직접 확인하십시오.

1. `HWP-Workbench-<버전>-arm64.dmg`를 열고 **HWP Workbench.app**을
   **Applications**로 끌어 놓습니다. 복사가 끝나면 DMG를 꺼냅니다.
2. 앱을 엽니다. 임시 서명(ad-hoc) 빌드는 공증되지 않았습니다.
   macOS 15 이상에서 차단되면 **시스템 설정 → 개인정보 보호 및 보안 →
   그래도 열기**를 선택합니다. 이 버전에서는 우클릭 **열기**로
   Gatekeeper를 우회할 수 없습니다. **첫 실행 절차는 아직 검증되지 않았습니다.**
   그래도 열리지 않으면 터미널에서 다운로드 격리 표시를 지웁니다.
   이 앱을 직접 받은 경우에만 실행합니다.

```sh
xattr -dr com.apple.quarantine "/Applications/HWP Workbench.app"
```

3. CLI를 PATH에 추가합니다. 터미널에서 아래 명령을 실행하고, 계속 쓰려면
   같은 줄을 `~/.zshrc`에 추가합니다.

```sh
export PATH="/Applications/HWP Workbench.app/Contents/Resources/bin:$PATH"
hwp-workbench capabilities
```

에이전트 연결: Claude Code와 Codex에서는 `hwp-workbench skill install`로
스킬을 설치할 수 있습니다. MCP 설정 명령을 출력하려면 아래 명령을
실행한 뒤 출력된 설정을 확인하고 적용합니다. 문서 폴더의 절대 경로를 씁니다.

```sh
hwp-workbench mcp-config --client claude-code --root /absolute/document-folder
hwp-workbench mcp-config --client codex --root /absolute/document-folder
hwp-workbench mcp-config --client json --root /absolute/document-folder
```

Claude Desktop에서는 마지막 명령의 `mcpServers` 블록을
`~/Library/Application Support/Claude/claude_desktop_config.json`에 합칩니다.
기존 설정을 먼저 백업하고 다른 서버 설정을 보존합니다. 앱을 다시 시작합니다.
CLI 경로는 `/Applications/HWP Workbench.app/Contents/Resources/bin/hwp-workbench`입니다.
**패키지의 CLI 실행 및 세 클라이언트 연결은 아직 검증되지 않았습니다.**

## English

This developer alpha needs an Apple Silicon Mac and macOS 12 or later.
Hancom reopen and document layout checks remain unverified.
Check saved files in Hancom 한글 before you submit an official document.

1. Open `HWP-Workbench-<version>-arm64.dmg`. Drag **HWP Workbench.app**
   to **Applications**, then eject the DMG.
2. Open the app. The ad-hoc build is not notarized. If macOS 15 or later
   blocks it, use **System Settings → Privacy & Security → Open Anyway**.
   Right-click **Open** no longer bypasses Gatekeeper on these versions.
   **This first-open procedure has not yet been tested.**
   If it still does not open, clear the download quarantine flag in Terminal.
   Do this only for a copy that you received directly from us:

```sh
xattr -dr com.apple.quarantine "/Applications/HWP Workbench.app"
```

3. Run the PATH command above. Add the same line to `~/.zshrc` for later
   terminal sessions. The CLI is at
   `/Applications/HWP Workbench.app/Contents/Resources/bin/hwp-workbench`.

Run `hwp-workbench skill install` for Claude Code and Codex skills.
Use the MCP commands above with an absolute document-folder path. Review and
apply the printed setup. For Claude Desktop, merge the JSON command's
`mcpServers` block into
`~/Library/Application Support/Claude/claude_desktop_config.json`, then restart
Claude Desktop. Back up the file first and keep other server entries.
**Packaged CLI execution and all three client connections remain unverified.**

## Developer packaging

Build offline with the approved Cargo wrapper:

```sh
HWP_WORKBENCH_CARGO_WRAPPER=/absolute/path/cargo-offline.sh npm run package:mac
```

Outputs go to `dist-mac/`. Existing outputs and checksum files are refused.
The final JSON reports decimal MB, both DMG compression sizes, SHA-256 values,
and the 60 MB app / 30 MB DMG budgets. Budget failures are reported without
failing the build. The app checksum covers the sorted bundle tree; its exact
encoding is in the JSON. A DMG checksum can be checked with `shasum -a 256 -c`.

`npm run package:mac -- --config /absolute/path/temporary.json` selects a config
copy. Resource source paths still refer to `app/src-tauri/`, not the copy's
folder. Script runtimes, Node modules, embedded frameworks and unexpected symlinks are refused.

For a later Developer ID build, use
`npm run package:mac -- --identity 'Developer ID Application: Name (TEAMID)'`.
This enables the hardened runtime and signing timestamp. **This option is
untested; notarization is not implemented.** Ad-hoc builds use neither option.
