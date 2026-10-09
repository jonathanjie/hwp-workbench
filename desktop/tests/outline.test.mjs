import {test} from 'node:test';
import assert from 'node:assert/strict';
import {headingNodes, nodeLocations} from '../navigation.js';

test('outline detects style and numbering, joins heading runs, and excludes table contents', () => {
  const nodes = [
    {id: 'a', text: 'Main ', role: 'body', paragraph: 1, style: {size: 20, bold: true}},
    {id: 'b', text: 'heading', role: 'body', paragraph: 1, style: {size: 20}},
    {id: 'c', text: 'normal body', role: 'body', paragraph: 2, style: {size: 10}},
    {id: 'd', text: '3-2-1. Policy', role: 'body', paragraph: 3, style: {size: 10}},
    {id: 'e', text: '가. 항목', role: 'body', paragraph: 4},
    {id: 'f', text: 'Table title', role: 'table-cell', style: {bold: true}}
  ];
  const headings = headingNodes(nodes);
  assert.deepEqual(headings.map(item => item.node.id), ['a', 'd', 'e']);
  assert.equal(headings[0].text, 'Main heading');
});

test('text locations retain sparse native table row and column addresses', () => {
  const locations = nodeLocations([{type: 'table', children: [{type: 'row', children: [{type: 'cell', address: {row: '2', column: '4'}, children: [{type: 'text', node: 'cell'}]}]}]}]);
  assert.equal(locations.get('cell'), 'T1 · r3 c5');
});
