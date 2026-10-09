import {textError} from './edit-model.js';

export function findMatches(model, query, caseSensitive = false) {
  if (!model || !query) {
    return [];
  }
  const literal = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const expression = new RegExp(literal, caseSensitive ? 'gu' : 'giu');
  const results = [];
  for (const node of model.nodes.values()) {
    const text = model.current(node);
    for (const match of text.matchAll(expression)) {
      results.push({node, text, start: match.index, end: match.index + match[0].length});
    }
  }
  return results;
}

export function replaceMatches(model, matches, replacement) {
  const groups = new Map();
  for (const match of matches) {
    if (!groups.has(match.node.id)) {
      groups.set(match.node.id, []);
    }
    groups.get(match.node.id).push(match);
  }
  const changes = [];
  const skipped = [];
  let replaced = 0;
  for (const group of groups.values()) {
    const node = model.nodes.get(group[0].node.id);
    const current = node && model.current(node);
    let reason = !node?.editable ? 'locked' : group.some(match => match.text !== current) ? 'sourceChanged' : null;
    let text = current;
    if (!reason) {
      // Apply from the end so all UTF-16 offsets remain valid.
      for (const match of group.toSorted((a, b) => b.start - a.start)) {
        text = text.slice(0, match.start) + replacement + text.slice(match.end);
      }
      reason = textError(text, node.text, model.lengthChangingText);
    }
    if (reason) {
      skipped.push(...group.map(match => ({...match, reason})));
    } else if (text !== current) {
      changes.push({node, text});
      replaced += group.length;
    }
  }
  if (changes.length && !model.stageBatch(changes)) {
    return {replaced: 0, skipped: [...skipped, ...matches.filter(match => changes.some(change => change.node.id === match.node.id)).map(match => ({...match, reason: 'tooManyEdits'}))]};
  }
  return {replaced, skipped};
}
