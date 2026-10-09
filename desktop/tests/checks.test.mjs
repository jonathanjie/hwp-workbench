import {test} from 'node:test';
import assert from 'node:assert/strict';
import {featureChecks} from '../checks.js';

test('checks retain engine limits and unknown warnings without layout certification', () => {
  const warnings = [{code: 'LAYOUT_UNVERIFIED', severity: 'warning', message: 'unverified'},
    {code: 'NEW_FEATURE_RISK', severity: 'warning', message: 'original risk'}];
  const checks = featureChecks({capabilities: {textEdit: true, formattingEdit: false, structuralEdit: false}, warnings});
  assert.equal(checks.find(item => item.key === 'textEdits').state, 'ok');
  assert.equal(checks.find(item => item.key === 'formatting').state, 'off');
  assert.equal(checks.find(item => item.code === 'LAYOUT_UNVERIFIED').state, 'off');
  assert.equal(checks.find(item => item.code === 'NEW_FEATURE_RISK').warning.message, 'original risk');
});
