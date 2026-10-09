export function fuzzyScore(label, query) {
  const text = label.normalize('NFKC').toLocaleLowerCase();
  const needle = query.trim().normalize('NFKC').toLocaleLowerCase();
  if (!needle) {
    return 1;
  }
  if (text === needle) {
    return 1000;
  }
  const direct = text.indexOf(needle);
  if (direct !== -1) {
    return 500 - direct;
  }
  let position = -1;
  let gaps = 0;
  for (const character of needle) {
    const next = text.indexOf(character, position + 1);
    if (next === -1) {
      return 0;
    }
    gaps += next - position - 1;
    position = next;
  }
  return Math.max(1, 100 - gaps);
}

export const COMMAND_ALIASES = {
  open: ['문서 불러오기', '열기', 'Open file', 'Load'],
  save: ['저장하기', 'Save document'],
  saveCopy: ['다른 이름으로 저장', '사본 저장', 'Save As', 'Save copy'],
  undo: ['되돌리기', '실행 취소', 'Undo'],
  replace: ['찾아 바꾸기', '바꾸기', 'Find and replace', 'Replace'],
  charFormat: ['글자 모양', '글꼴', 'Character shape', 'Character format', 'Font'],
  paraFormat: ['문단 모양', 'Paragraph shape', 'Paragraph format'],
  pageFormat: ['편집 용지', 'Page setup', 'Paper'],
  styleFormat: ['스타일', 'Style'],
  symbols: ['문자표', 'Symbol', 'Character map'],
  controlMarks: ['조판 부호', 'Control marks', 'Formatting marks'],
  pages: ['쪽 윤곽', 'Page outline', 'Pages']
};

export function matchingCommands(commands, query) {
  return commands.map(command => ({command, score: Math.max(...[command.en, command.ko, ...(command.aliases || COMMAND_ALIASES[command.label || command.id] || [])].map(label => fuzzyScore(label, query)))}))
    .filter(item => item.score > 0)
    .sort((a, b) => b.score - a.score)
    .map(item => item.command);
}
