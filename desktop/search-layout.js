import {utf16Offset, readerText} from './native-text.js';

// Layout charX arrays give native glyph positions. Do not estimate glyph widths
// from character counts, which is inaccurate for Korean and mixed-font text.
export function matchBounds(run, node, match, includeDeletion = false) {
  if (match.node.id !== node.id || match.text !== node.text) {
    return null;
  }
  const text = readerText(node);
  const matchStart = readerText(node, node.text.slice(0, match.start)).length;
  const matchEnd = readerText(node, node.text.slice(0, match.end)).length;
  const nativeOffset = run.charStart - (node.anchor?.charOffset || 0);
  let offset = utf16Offset(text, nativeOffset);
  if (text.slice(offset, offset + run.text.length) !== run.text) {
    const first = text.indexOf(run.text);
    if (first === -1 || first !== text.lastIndexOf(run.text)) {
      return null;
    }
    offset = first;
  }
  const start = Math.max(matchStart, offset) - offset;
  const end = Math.min(matchEnd, offset + run.text.length) - offset;
  const deletion = includeDeletion && matchStart === matchEnd && matchStart >= offset && matchStart <= offset + run.text.length;
  if (end <= start && !deletion) {
    return null;
  }
  const from = [...run.text.slice(0, start)].length;
  const to = [...run.text.slice(0, end)].length;
  if (!run.charX || run.charX[from] === undefined || run.charX[to] === undefined) {
    return null;
  }
  return {x: run.x + run.charX[from], y: run.y, w: deletion ? 2 : run.charX[to] - run.charX[from], h: run.h};
}
