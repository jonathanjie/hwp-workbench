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

export function changeCards(model, onRevert) {
  return Object.entries(model.edits).map(([id, text], index) => {
    const node = model.nodes.get(id);
    const card = document.createElement('article');
    card.className = 'change';
    card.dataset.nodeId = id;
    const header = document.createElement('div');
    header.className = 'head';
    const number = document.createElement('span');
    number.className = 'num';
    number.textContent = index + 1;
    header.append(number, document.createTextNode(`P${(node.anchor?.paragraph ?? node.paragraph ?? 0) + (node.anchor ? 1 : 0)} · ${roleLabel(node.role)}`));
    const diff = document.createElement('div');
    diff.className = 'diff';
    const words = wordDiff(node.text, text);
    const deleted = document.createElement('del');
    deleted.textContent = words.removed;
    const inserted = document.createElement('ins');
    inserted.textContent = words.added;
    diff.append(document.createTextNode(words.prefix), deleted, inserted, document.createTextNode(words.suffix));
    card.append(header, diff);
    if (onRevert) {
      const actions = document.createElement('div');
      actions.className = 'acts';
      const revert = document.createElement('button');
      revert.className = 'btn ghost';
      revert.textContent = t('revert');
      revert.onclick = () => onRevert(id);
      actions.append(revert);
      card.append(actions);
    }
    return card;
  });
}
