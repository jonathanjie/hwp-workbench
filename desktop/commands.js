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

export function matchingCommands(commands, query) {
  return commands.map(command => ({command, score: Math.max(fuzzyScore(command.en, query), fuzzyScore(command.ko, query))}))
    .filter(item => item.score > 0)
    .sort((a, b) => b.score - a.score)
    .map(item => item.command);
}
