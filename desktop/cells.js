// Native engine addresses supply reading order; page geometry never selects cells.
export function nativeCells(view, nodes) {
  const byId = new Map(nodes.map(node => [node.id, node]));
  const result = [];
  function textIds(item) {
    if (item.type === 'table') {
      return [];
    }
    return item.type === 'text' ? [item.node] : (item.children || []).flatMap(textIds);
  }
  function tableCells(item) {
    if (item.type === 'cell') {
      return [item];
    }
    if (item.type === 'table') {
      return [];
    }
    return (item.children || []).flatMap(tableCells);
  }
  function visit(item) {
    if (item.type === 'table') {
      const cells = (item.children || []).flatMap(tableCells).filter(cell => {
        const {row, column} = cell.address || {};
        return /^\d+$/.test(row) && /^\d+$/.test(column);
      }).sort((a, b) => Number(a.address.row) - Number(b.address.row) || Number(a.address.column) - Number(b.address.column));
      for (const cell of cells) {
        const members = textIds(cell).map(id => byId.get(id)).filter(Boolean);
        const node = members.find(node => node.editable) || members[0];
        if (node) {
          result.push({node, ids: members.map(node => node.id), address: cell.address});
        }
      }
    }
    (item.children || []).forEach(visit);
  }
  view.forEach(visit);
  return result;
}
export function adjacentCell(cells, nodeId, direction) {
  const skipped = [];
  const start = cells.findIndex(cell => cell.ids.includes(nodeId));
  if (start === -1) {
    return {node: null, skipped};
  }
  for (let index = start + direction; index >= 0 && index < cells.length; index += direction) {
    const {node} = cells[index];
    if (node.editable) {
      return {node, skipped};
    }
    skipped.push({nodeId: node.id, reason: node.lockReason || 'locked'});
  }
  return {node: null, skipped};
}

export function stageCellBeforeMove(slip) {
  if (!slip.dirty) {
    return true;
  }
  slip.update();
  if (slip.stageButton.disabled || !slip.onStage(slip.node, slip.editor.value)) {
    return false;
  }
  // At the final cell, Tab stages without moving. Keep that slip clean too.
  slip.initial = slip.editor.value;
  slip.update();
  return true;
}
