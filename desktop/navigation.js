import {sectionIndex} from './hit-map.js';

export function headingNodes(nodes) {
  const sizes = nodes.filter(node => node.role === 'body' && node.style?.size).map(node => node.style.size).sort((a, b) => a - b);
  const bodySize = sizes.length ? sizes[Math.floor(sizes.length / 2)] : 10;
  const groups = new Map();
  for (const node of nodes) {
    if (node.role !== 'body') {
      continue;
    }
    const key = `${sectionIndex(node)}:${node.anchor?.paragraph ?? node.paragraph ?? node.id}`;
    if (!groups.has(key)) {
      groups.set(key, {node, nodes: [], text: ''});
    }
    const group = groups.get(key);
    group.nodes.push(node);
    group.text += node.text;
  }
  return [...groups.values()].filter(group => group.text.trim() && group.text.length <= 200 &&
    (group.nodes.some(node => node.style?.bold || node.style?.size >= bodySize * 1.2) ||
    /^(?:\d+(?:[-.]\d+)*[.)]|[가-힣][.)])\s*/u.test(group.text.trim())));
}

export function nodeLocations(view) {
  const locations = new Map();
  let tables = 0;
  function walk(item, table = null, cell = null) {
    if (item.type === 'table') {
      tables += 1;
      table = tables;
    }
    if (item.type === 'cell') {
      const row = Number(item.address?.row);
      const column = Number(item.address?.column);
      cell = Number.isInteger(row) && row >= 0 && Number.isInteger(column) && column >= 0 ? {row, column} : null;
    }
    if (item.type === 'text' && table && cell) {
      locations.set(item.node, `T${table} · r${cell.row + 1} c${cell.column + 1}`);
    }
    for (const child of item.children || []) {
      walk(child, table, cell);
    }
  }
  for (const item of view) {
    walk(item);
  }
  return locations;
}
