import {test} from 'node:test';
import assert from 'node:assert/strict';
import {nativeCells, adjacentCell} from '../cells.js';
const cell = (id, row, column) => ({type: 'cell', address: {row: String(row), column: String(column)},
  children: [{type: 'paragraph', children: [{type: 'text', node: id}]}]});
test('cell Tab order uses native addresses, skips locks with reason and supports reverse', () => {
  const nodes = [{id: 'c', editable: true}, {id: 'b', editable: false, lockReason: 'field'}, {id: 'a', editable: true}];
  const view = [{type: 'table', children: [cell('c', 1, 0), cell('b', 0, 1), cell('a', 0, 0)]}];
  const cells = nativeCells(view, nodes);
  assert.deepEqual(cells.map(item => item.node.id), ['a', 'b', 'c']);
  const next = adjacentCell(cells, 'a', 1);
  assert.equal(next.node.id, 'c');
  assert.deepEqual(next.skipped, [{nodeId: 'b', reason: 'field'}]);
  assert.equal(adjacentCell(cells, 'c', -1).node.id, 'a');
  assert.equal(adjacentCell(cells, 'c', 1).node, null);
  assert.equal(nativeCells([{type: 'table', children: [{type: 'cell', children: [{type: 'text', node: 'a'}]}]}], nodes).length, 0);
});

test('a staged final cell becomes clean and invalid cell text stays unstaged', async () => {
  const {stageCellBeforeMove} = await import('../cells.js');
  const {EditModel, textError} = await import('../edit-model.js');
  const node = {id: 'cell', text: '기존', editable: true};
  const model = new EditModel({nodes: [node], capabilities: {lengthChangingText: true}});
  const slip = {initial: node.text, node, editor: {value: '새 내용'}, stageButton: {},
    get dirty() { return this.initial !== this.editor.value; },
    update() { this.stageButton.disabled = !!textError(this.editor.value, node.text, true); },
    onStage: (target, text) => model.stage(target, text)};
  assert.equal(stageCellBeforeMove(slip), true);
  assert.equal(slip.dirty, false);
  assert.equal(model.current(node), '새 내용');
  slip.editor.value = 'bad\ntext';
  assert.equal(stageCellBeforeMove(slip), false);
  assert.equal(model.current(node), '새 내용');
});
