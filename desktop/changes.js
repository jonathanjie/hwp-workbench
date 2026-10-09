import {t, roleLabel} from './i18n.js';

export function wordDiff(before, after) {
  const original = before.match(/\s+|[^\s]+/gu) || [];
  const changed = after.match(/\s+|[^\s]+/gu) || [];
  let prefix = 0;
  while (prefix < original.length && prefix < changed.length && original[prefix] === changed[prefix]) {
    prefix += 1;
  }
  let suffix = 0;
  while (suffix < original.length - prefix && suffix < changed.length - prefix &&
         original[original.length - 1 - suffix] === changed[changed.length - 1 - suffix]) {
    suffix += 1;
  }
  return {
    prefix: original.slice(0, prefix).join(''),
    removed: original.slice(prefix, original.length - suffix).join(''),
    added: changed.slice(prefix, changed.length - suffix).join(''),
    suffix: suffix ? original.slice(-suffix).join('') : ''
  };
}

export function wordDiffSegments(before, after) {
  const original = before.match(/\s+|[^\s]+/gu) || [];
  const changed = after.match(/\s+|[^\s]+/gu) || [];
  // Bound the comparison matrix for long document nodes. The fallback still
  // retains all text and compares only the changed middle word range.
  if (original.length * changed.length > 65536) {
    const diff = wordDiff(before, after);
    return [{type: 'same', text: diff.prefix}, {type: 'del', text: diff.removed},
      {type: 'ins', text: diff.added}, {type: 'same', text: diff.suffix}];
  }
  const lengths = Array.from({length: original.length + 1}, () => new Uint32Array(changed.length + 1));
  for (let i = original.length - 1; i >= 0; i -= 1) {
    for (let j = changed.length - 1; j >= 0; j -= 1) {
      lengths[i][j] = original[i] === changed[j] ? lengths[i + 1][j + 1] + 1 : Math.max(lengths[i + 1][j], lengths[i][j + 1]);
    }
  }
  const segments = [];
  function append(type, text) {
    const previous = segments.at(-1);
    if (previous?.type === type) {
      previous.text += text;
    } else {
      segments.push({type, text});
    }
  }
  let i = 0;
  let j = 0;
  while (i < original.length || j < changed.length) {
    if (i < original.length && j < changed.length && original[i] === changed[j]) {
      append('same', original[i]);
      i += 1;
      j += 1;
    } else if (j >= changed.length || (i < original.length && lengths[i + 1][j] >= lengths[i][j + 1])) {
      append('del', original[i]);
      i += 1;
    } else {
      append('ins', changed[j]);
      j += 1;
    }
  }
  return segments;
}

export function changeCards(model, onRevert, ids = Object.keys(model.edits), actionLabel = 'revert') {
  return ids.map((id, index) => {
    const node = model.nodes.get(id);
    const text = model.edits[id];
    const card = document.createElement('article');
    const author = model.origins?.[id];
    card.className = author ? 'change agent' : 'change';
    card.dataset.nodeId = id;
    const header = document.createElement('div');
    header.className = 'head';
    const number = document.createElement('span');
    number.className = 'num';
    number.textContent = index + 1;
    header.append(number, document.createTextNode(`P${(node.anchor?.paragraph ?? node.paragraph ?? 0) + (node.anchor ? 1 : 0)} · ${roleLabel(node.role)}`));
    if (author) {
      const who = document.createElement('span');
      who.className = 'who';
      who.textContent = author;
      header.append(who);
    }
    const diff = document.createElement('div');
    diff.className = 'diff';
    for (const segment of wordDiffSegments(node.text, text)) {
      if (!segment.text) {
        continue;
      }
      if (segment.type === 'same') {
        diff.append(document.createTextNode(segment.text));
      } else {
        const mark = document.createElement(segment.type);
        mark.textContent = segment.text;
        diff.append(mark);
      }
    }
    card.append(header, diff);
    if (onRevert) {
      const actions = document.createElement('div');
      actions.className = 'acts';
      const revert = document.createElement('button');
      revert.className = 'btn ghost';
      revert.textContent = t(actionLabel);
      revert.onclick = () => onRevert(id);
      actions.append(revert);
      card.append(actions);
    }
    return card;
  });
}
