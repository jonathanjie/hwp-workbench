const strings = {
  en: {
    keyScheme: 'Keyboard scheme', schemeWindowsChoice: 'Windows Hangul', schemeMacChoice: 'Mac', schemeWindows: 'Keyboard · Windows Hangul', schemeMac: 'Keyboard · Mac', selectAll: 'Select all', nextCell: 'Next cell', previousCell: 'Previous cell', cellSlip: 'Cell block', palette: 'Commands',
    charFormat: 'Character shape', paraFormat: 'Paragraph shape', pageFormat: 'Page setup', styleFormat: 'Style', symbols: 'Character map', controlMarks: 'Control marks',
    readOnly: 'Read only', font: 'Font', size: 'Size', widthRatio: 'Width ratio (%)', letterSpacing: 'Letter spacing (%)', bold: 'Bold', italic: 'Italic', underline: 'Underline', colour: 'Colour', alignment: 'Alignment', indent: 'Indent (pt)', before: 'Before (pt)', after: 'After (pt)', lineSpacing: 'Line spacing', width: 'Width', height: 'Height', orientation: 'Orientation', left: 'Left', right: 'Right', top: 'Top', bottom: 'Bottom', header: 'Header', footer: 'Footer', gutter: 'Gutter', name: 'Name', on: 'On', formatOff: 'Off', portrait: 'Portrait', landscape: 'Landscape',
    symbolSearch: 'Search symbols',
    cellsSkipped: 'Skipped cells · {n} · {reason}',
    fontInstall: 'Hamchorom fonts',
    center: 'Centre', justify: 'Justify',
    save: 'Save', savedInPlace: 'Saved',
    warningDetails: 'Warnings', fonts: 'Fonts', tables: 'Tables', images: 'Images', previewCache: 'Preview cache', sameLengthFeature: 'Same length', scriptStub: 'Default script', textEdits: 'Text edits', formatting: 'Formatting', structureEdits: 'Structure edits', ok: 'OK', partial: 'Partial', off: 'Off', noHeadings: 'No headings', recentFailed: 'Recent files unavailable', thumbnailFailed: 'Thumbnail unavailable',
    agents: 'Agents',
    local: 'Local',
    install: 'Install',
    skill: 'Skill',
    unavailable: 'Unavailable',
    unverified: 'Unverified',
    copyMcp: 'Copy MCP',
    showBundle: 'Show bundle',
    skillInstalled: 'Skill installed',
    copied: 'Copied',
    agentLink: 'Agent link',
    agentCount: 'Agent · {n}',
    agentProposed: '{author} proposed {n} edits',
    agentLinkFailed: 'Agent link unavailable · retry',
    agentRead: 'Read',
    agentProposal: 'Proposed',
    agentWaiting: 'Waiting',
    apply: 'Apply',
    decline: 'Decline',
    applyAll: 'Apply all',
    declineAll: 'Decline all',
    accepted: 'Accepted',
    declined: 'Declined',
    proposalPartial: 'Partial',
    stale: 'Stale · reopen',
    unreadable: 'Unreadable proposal',
    needsReview: 'Needs review',
    timeUnavailable: 'Time unavailable',
    proposalConflict: 'Staged edit conflicts · revert first',
    livePreviewFailed: 'Preview failed · retry',
    reviewSummary: '{n} edits · {pages} pages',
    replacedToast: '{n} replaced in {paragraphs} paragraphs',
    stagedToast: 'Staged · {n}', remove: 'Remove', pageLabel: 'Page {n}', unlocated: 'Page unavailable',
    previousMatch: 'Previous match', nextMatch: 'Next match', matchCase: 'Match case', replacement: 'Replacement', replaceOne: 'Replace', replaceAll: 'Replace all', skipped: 'Skipped · {n}', tooManyEdits: 'Edit limit exceeded',
    paletteQuery: 'Search commands and text', all: 'All', commands: 'Commands', commandGroup: 'COMMANDS', documentGroup: 'IN DOCUMENT', noResults: 'No matches', navigate: 'Move', run: 'Open', scope: 'Scope', find: 'Find', replace: 'Find and replace…', shortcuts: 'Keyboard shortcuts', theme: 'Theme', themeSystem: 'Theme · System', themeLight: 'Theme · Light', themeDark: 'Theme · Dark', togglePanel: 'Panel', stageShortcut: 'Stage text', closeShortcut: 'Close', findNextShortcut: 'Next match', findPreviousShortcut: 'Previous match', paletteScopes: 'Palette scope', paletteMove: 'Palette selection', paletteRun: 'Run command', panelTabs: 'Panel tabs', system: 'System', light: 'Light', dark: 'Dark',
    open: 'Open…', sample: 'Sample', recent: 'Recent', saveCopy: 'Save As',
    edits: '{n} edits', edit: '1 edit', pages: 'Pages', outline: 'Outline',
    revert: 'Revert', stage: 'Stage', changes: 'Changes', agent: 'Agent', checks: 'Checks',
    staged: 'Staged · {n}', reviewAll: 'Review all', preview: 'rhwp 0.8.7 preview',
    structure: 'Structure view', locked: 'Locked', sameLength: 'Same length · {a}/{b}',
    layoutUnverified: 'Hancom layout unverified',
    ack: "I'll check the saved document in Hancom before submitting",
    saved: 'Saved {name}', showInFinder: 'Show in Finder', openFailed: "Couldn't open · {reason}",
    failed: "Couldn't complete · {reason}", close: 'Close', cancel: 'Cancel', language: 'Language',
    zoomOut: 'Zoom out', zoomIn: 'Zoom in', fitWidth: 'Fit width',
    agentPending: 'Agent · pending', fontSubstitutions: 'Font substitutions',
    noChanges: 'No edits', empty: 'Empty', preserved: 'Preserved · read-only object',
    body: 'Body', tableCell: 'Table cell', headerFooter: 'Header / footer', text: 'Text',
    undo: 'Undo', redo: 'Redo', copyId: 'Copy node ID', exportPatch: 'Export patch',
    controls: 'Plain text only', inlineMarker: 'Keep or delete existing markers; do not add or copy a marker.', lengthError: 'Same UTF-16 length required', tooLong: 'Text exceeds 200,000 characters',
    restore: 'Restore staged draft?', discard: 'Discard unsaved text?', unstaged: 'Unstaged text',
    busy: 'Processing…', bridge: 'Desktop bridge unavailable', sourceChanged: 'Source changed · reopen',
    notHwp: 'not an HWP 5 file', invalidDocument: 'unsupported or invalid document',
    tooLarge: 'limit exceeded', existingOutput: 'copy exists · choose a new name',
    python: 'Python unavailable', timeout: 'processing timed out · retry',
    engineFailed: 'document operation failed', previewFailed: 'Page preview unavailable · structure view',
    fontRisk: 'Font identity and metrics unverified · no bundled fonts',
    tableRisk: 'Tables retained · full layout and structure editing unavailable',
    imageRisk: 'Images retained · object editing unavailable',
    cacheRisk: 'Original previews and line caches retained · may be stale after edits',
    hwpRisk: 'Same UTF-16 length only · structure editing blocked',
    hwpViewRisk: 'Engine HWP structure view · table, control and style streams retained',
    protectedRisk: 'Protected or active content · edits blocked · code not executed',
    objectRisk: '{object} retained · full layout and behavior unverified · editing unavailable',
    equations: 'Equations', ole: 'OLE objects', charts: 'Charts', headers: 'Headers / footers',
    footnotes: 'Footnotes / endnotes', fields: 'Fields', shapes: 'Shapes', numbering: 'Page numbering',
    bookmarks: 'Bookmarks', defaultScript: 'Default script stub retained · not executed', unknownRisk: 'Native payload retained · compatibility unverified'
  },
  ko: {
    keyScheme: '단축키 방식', schemeWindowsChoice: 'Windows 한글', schemeMacChoice: 'Mac', schemeWindows: '단축키 · Windows 한글', schemeMac: '단축키 · Mac', selectAll: '모두 선택', nextCell: '다음 셀', previousCell: '이전 셀', cellSlip: '셀 블록', palette: '명령',
    charFormat: '글자 모양', paraFormat: '문단 모양', pageFormat: '편집 용지', styleFormat: '스타일', symbols: '문자표', controlMarks: '조판 부호',
    readOnly: '읽기 전용', font: '글꼴', size: '크기', widthRatio: '장평 (%)', letterSpacing: '자간 (%)', bold: '진하게', italic: '기울임', underline: '밑줄', colour: '글자 색', alignment: '정렬', indent: '첫 줄 (pt)', before: '문단 위 (pt)', after: '문단 아래 (pt)', lineSpacing: '줄 간격', width: '폭', height: '길이', orientation: '용지 방향', left: '왼쪽', right: '오른쪽', top: '위쪽', bottom: '아래쪽', header: '머리말', footer: '꼬리말', gutter: '제본 여백', name: '이름', on: '켜짐', formatOff: '꺼짐', portrait: '세로', landscape: '가로',
    symbolSearch: '문자 검색',
    cellsSkipped: '건너뛴 셀 · {n} · {reason}',
    fontInstall: '함초롬 글꼴',
    center: '가운데', justify: '양쪽',
    save: '저장', savedInPlace: '저장됨',
    warningDetails: '경고', fonts: '글꼴', tables: '표', images: '이미지', previewCache: '미리보기 캐시', sameLengthFeature: '길이 유지', scriptStub: '기본 스크립트', textEdits: '본문 수정', formatting: '서식', structureEdits: '구조 수정', ok: '가능', partial: '부분', off: '미지원', noHeadings: '제목 없음', recentFailed: '최근 문서 사용 불가', thumbnailFailed: '쪽 미리보기 사용 불가',
    agents: '에이전트',
    local: '로컬',
    install: '설치',
    skill: '스킬',
    unavailable: '사용 불가',
    unverified: '미검증',
    copyMcp: 'MCP 복사',
    showBundle: '번들 보기',
    skillInstalled: '스킬 설치됨',
    copied: '복사됨',
    agentLink: '에이전트 연결',
    agentCount: '에이전트 · {n}',
    agentProposed: '{author}이(가) 수정 {n}건을 제안함',
    agentLinkFailed: '에이전트 연결 불가 · 다시 시도',
    agentRead: '읽기',
    agentProposal: '제안',
    agentWaiting: '검토 대기',
    apply: '적용',
    decline: '거절',
    applyAll: '모두 적용',
    declineAll: '모두 거절',
    accepted: '적용됨',
    declined: '거절됨',
    proposalPartial: '일부 적용',
    stale: '이전 버전 · 다시 불러오기',
    unreadable: '제안 읽기 불가',
    needsReview: '다시 검토 필요',
    timeUnavailable: '시간 정보 없음',
    proposalConflict: '적용된 수정과 충돌 · 먼저 되돌리기',
    livePreviewFailed: '미리보기 실패 · 다시 시도',
    reviewSummary: '수정 {n}건 · {pages}쪽',
    replacedToast: '문단 {paragraphs}개에서 {n}건 바꿈',
    stagedToast: '수정 적용 · {n}', remove: '삭제', pageLabel: '{n}쪽', unlocated: '쪽 위치 없음',
    previousMatch: '이전 일치', nextMatch: '다음 일치', matchCase: '대소문자 구분', replacement: '바꿀 내용', replaceOne: '바꾸기', replaceAll: '모두 바꾸기', skipped: '건너뜀 · {n}', tooManyEdits: '수정 한도 초과',
    paletteQuery: '명령 및 본문 검색', all: '전체', commands: '명령', commandGroup: '명령', documentGroup: '문서 본문', noResults: '일치 없음', navigate: '이동', run: '불러오기', scope: '범위', find: '찾기', replace: '찾아 바꾸기…', shortcuts: '키보드 단축키', theme: '테마', themeSystem: '테마 · 시스템', themeLight: '테마 · 밝게', themeDark: '테마 · 어둡게', togglePanel: '패널', stageShortcut: '본문 수정 적용', closeShortcut: '닫기', findNextShortcut: '다음 일치', findPreviousShortcut: '이전 일치', paletteScopes: '명령 범위', paletteMove: '명령 선택', paletteRun: '명령 실행', panelTabs: '패널 탭', system: '시스템', light: '밝게', dark: '어둡게',
    open: '불러오기…', sample: '샘플', recent: '최근', saveCopy: '다른 이름으로 저장',
    edits: '수정 {n}', edit: '수정 1', pages: '쪽', outline: '개요',
    revert: '되돌리기', stage: '수정 적용', changes: '변경', agent: '에이전트', checks: '호환성',
    staged: '수정 적용 · {n}', reviewAll: '모두 검토', preview: 'rhwp 0.8.7 미리보기',
    structure: '구조 보기', locked: '잠김', sameLength: '길이 유지 · {a}/{b}',
    layoutUnverified: '한컴 배치 미검증', ack: '제출 전 한컴에서 저장한 문서를 확인하겠습니다',
    saved: '{name} 저장됨', showInFinder: 'Finder에서 보기', openFailed: '열 수 없음 · {reason}',
    failed: '완료할 수 없음 · {reason}', close: '닫기', cancel: '취소', language: '언어',
    zoomOut: '축소', zoomIn: '확대', fitWidth: '너비 맞춤',
    agentPending: '에이전트 · 준비 중', fontSubstitutions: '글꼴 대체',
    noChanges: '수정 없음', empty: '비어 있음', preserved: '보존됨 · 읽기 전용 개체',
    body: '본문', tableCell: '표 셀', headerFooter: '머리말 / 꼬리말', text: '텍스트',
    undo: '되돌리기', redo: '다시 실행', copyId: '노드 ID 복사', exportPatch: '패치 내보내기',
    controls: '일반 텍스트만 지원', inlineMarker: '기존 표시는 유지하거나 삭제하세요. 추가하거나 복사할 수 없습니다.', lengthError: '동일한 UTF-16 길이 필요', tooLong: '텍스트 200,000자 초과',
    restore: '수정 초안 복원?', discard: '미저장 텍스트 삭제?', unstaged: '미적용 텍스트',
    busy: '처리 중…', bridge: '데스크톱 연결 없음', sourceChanged: '원본 변경됨 · 다시 불러오기',
    notHwp: 'HWP 5 파일 아님', invalidDocument: '미지원 또는 잘못된 문서',
    tooLarge: '한도 초과', existingOutput: '사본 존재 · 새 이름 필요',
    python: 'Python 사용 불가', timeout: '처리 시간 초과 · 재시도',
    engineFailed: '문서 작업 실패', previewFailed: '쪽 미리보기 사용 불가 · 구조 보기',
    fontRisk: '글꼴 식별 및 폭 미검증 · 포함된 글꼴 없음',
    tableRisk: '표 보존 · 전체 배치 및 구조 편집 미지원',
    imageRisk: '이미지 보존 · 개체 편집 미지원',
    cacheRisk: '원본 미리보기 및 줄 캐시 유지 · 수정 후 오래된 내용 가능',
    hwpRisk: '동일한 UTF-16 길이만 지원 · 구조 편집 차단',
    hwpViewRisk: '엔진 HWP 구조 보기 · 표, 제어 및 스타일 스트림 보존',
    protectedRisk: '보호 또는 활성 콘텐츠 · 편집 차단 · 코드 실행 안 함',
    objectRisk: '{object} 보존 · 전체 배치 및 동작 미검증 · 편집 미지원',
    equations: '수식', ole: 'OLE 개체', charts: '차트', headers: '머리말 / 꼬리말',
    footnotes: '각주 / 미주', fields: '필드', shapes: '도형', numbering: '쪽 번호',
    bookmarks: '책갈피', defaultScript: '기본 스크립트 보존 · 실행 안 함', unknownRisk: '네이티브 데이터 보존 · 호환성 미검증'
  }
};
let language = typeof window === 'undefined' ? null : localStorage.getItem('hwp-language');
if (!['en', 'ko'].includes(language)) {
  language = (globalThis.navigator?.language || 'en').toLowerCase().startsWith('ko') ? 'ko' : 'en';
}

export function currentLanguage() {
  return language;
}

export function labelFor(locale, key) {
  return strings[locale][key] || key;
}

export function t(key, values = {}) {
  const template = strings[language][key] || key;
  return template.replace(/\{(\w+)\}/g, (_, name) => values[name] ?? '');
}

export function applyLanguage() {
  document.documentElement.lang = language;
  for (const element of document.querySelectorAll('[data-label]')) {
    element.textContent = t(element.dataset.label);
  }
  for (const element of document.querySelectorAll('[data-aria]')) {
    element.setAttribute('aria-label', t(element.dataset.aria));
  }
  for (const element of document.querySelectorAll('[data-title]')) {
    element.title = t(element.dataset.title);
    element.setAttribute('aria-label', t(element.dataset.title));
  }
}

export function toggleLanguage() {
  language = language === 'en' ? 'ko' : 'en';
  localStorage.setItem('hwp-language', language);
  applyLanguage();
}

export function roleLabel(role) {
  return t({'body': 'body', 'table-cell': 'tableCell', 'header-footer': 'headerFooter'}[role] || 'text');
}

export function warningLabel(warning) {
  const common = {
    LAYOUT_UNVERIFIED: 'layoutUnverified', FONT_METRICS_UNVERIFIED: 'fontRisk',
    TABLE_LAYOUT: 'tableRisk', IMAGES: 'imageRisk', PREVIEW_CACHE_RETAINED: 'cacheRisk',
    HWP_EQUAL_LENGTH_ONLY: 'hwpRisk', HWP_LAYOUT_NOT_RENDERED: 'hwpViewRisk',
    PROTECTED_OR_ACTIVE_CONTENT: 'protectedRisk', DEFAULT_SCRIPT_STUB: 'defaultScript'
  };
  const objects = {EQUATIONS: 'equations', OLE_OBJECTS: 'ole', CHARTS: 'charts',
    HEADERS_FOOTERS: 'headers', FOOTNOTES: 'footnotes', FIELDS: 'fields', SHAPES: 'shapes',
    PAGE_NUMBERING: 'numbering', BOOKMARKS: 'bookmarks'};
  if (common[warning.code]) {
    return t(common[warning.code]);
  }
  if (objects[warning.code]) {
    return t('objectRisk', {object: t(objects[warning.code])});
  }
  return t('unknownRisk');
}

export function errorReason(error) {
  const message = String(error?.message || error);
  if (/source changed|stale|source path changed/i.test(message)) {
    return t('sourceChanged');
  }
  if (/exist|overwrite/i.test(message)) {
    return t('existingOutput');
  }
  if (/not.*HWP|HWP 5|compound file/i.test(message)) {
    return t('notHwp');
  }
  if (/limit|too large|64 MiB/i.test(message)) {
    return t('tooLarge');
  }
  if (/Python/i.test(message)) {
    return t('python');
  }
  if (/timed out/i.test(message)) {
    return t('timeout');
  }
  if (/unsupported|invalid|corrupt|missing|XML|deflate/i.test(message)) {
    return t('invalidDocument');
  }
  return t('engineFailed');
}
