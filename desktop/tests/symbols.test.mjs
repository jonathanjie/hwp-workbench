import {test} from 'node:test';
import assert from 'node:assert/strict';
import {searchSymbols, insertSymbol, rememberSymbol} from '../symbols.js';

test('symbols search both languages, insert at selection, and bound recents', () => {
  assert.equal(searchSymbols('참고표')[0].symbol, '※');
  assert.equal(searchSymbols('reference')[0].symbol, '※');
  assert.ok(searchSymbols('동그라미').some(item => item.symbol === '○'));
  assert.deepEqual(insertSymbol('가나다', 1, 2, '※'), {text: '가※다', caret: 2});
  assert.deepEqual(rememberSymbol(['○', '※'], '※'), ['※', '○']);
  assert.equal(rememberSymbol(Array(30).fill('○'), '※').length, 2);
});
