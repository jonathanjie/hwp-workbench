// Native layout offsets count Unicode scalars. DOM text ranges count UTF-16.
export function readerText(node, text = node.text) {
  if (!node.inlineMarkers) {
    return text;
  }
  return text.replace(/[\ufdd0-\ufdef]/gu, marker => node.inlineMarkers[marker] ?? marker);
}

export function nativeLength(text) {
  return [...text].length;
}

export function nativeSlice(text, start, length) {
  return [...text].slice(start, start + length).join('');
}

export function utf16Offset(text, nativeOffset) {
  return [...text].slice(0, nativeOffset).join('').length;
}
