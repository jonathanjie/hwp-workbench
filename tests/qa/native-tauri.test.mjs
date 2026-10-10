import test from 'node:test';
import assert from 'node:assert/strict';
import * as qa from '../../scripts/qa/native-tauri.mjs';

test('arguments default to the package and reject unknown or invalid options', () => {
  const options = qa.parseArguments([], '/project');
  assert.equal(options.app, '/project/dist-mac/HWP Workbench.app');
  assert.equal(options.dryRun, false);
  assert.equal(options.rounds, 3);
  assert.equal(qa.parseArguments(['--dry-run', '--rounds', '1'], '/project').dryRun, true);
  for (const args of [['--typo'], ['--app'], ['--timeout', 'NaN'], ['--rounds', '0'], ['--rounds', '100']]) {
    assert.throws(() => qa.parseArguments(args, '/project'));
  }
});

test('IME comparison is exact, including newlines, null and staged arrays', () => {
  assert.equal(qa.compareResult({value: '한글'}, {value: '한글'}), true);
  assert.equal(qa.compareResult({value: '한글\n'}, {value: '한글'}), false);
  assert.equal(qa.compareResult({value: null, staged: ['한글']}, {value: null, staged: ['한글']}), true);
  assert.equal(qa.compareResult({value: null, staged: ['한글', '한글']}, {value: null, staged: ['한글']}), false);
  assert.equal(qa.compareResult({error: 'AX unavailable'}, {value: null}), false);
});

const ready = {platform: 'darwin', screenUnlocked: true, trusted: true, koreanAvailable: true,
  currentSource: 'original', appExists: true, cliExists: true, fixturesExist: true, instances: []};
test('preflight refuses a locked or unknown session, missing permission, source or existing app', () => {
  assert.deepEqual(qa.preflightFailures(ready), []);
  for (const field of ['screenUnlocked', 'trusted', 'koreanAvailable', 'appExists', 'cliExists', 'fixturesExist']) {
    assert.ok(qa.preflightFailures({...ready, [field]: false}).length, field);
  }
  assert.ok(qa.preflightFailures({...ready, screenUnlocked: null}).length);
  assert.ok(qa.preflightFailures({...ready, currentSource: ''}).length);
  assert.ok(qa.preflightFailures({...ready, platform: 'linux'}).length);
  assert.ok(qa.preflightFailures({...ready, instances: [{pid: 12}]}).length);
});

test('the dry-run plan has only read-only helper commands', () => {
  assert.deepEqual(qa.PREFLIGHT_COMMANDS, [['preflight']]);
  assert.ok(qa.driverPlan().some(step => step.includes('Cmd+S')));
});

test('memory ownership uses parent chains or an explicit responsible pid, never a new PID alone', () => {
  const rows = qa.parseProcesses('10 1 100 /bundle/HWP Workbench\n20 1 80 com.apple.WebKit.WebContent\n30 10 50 com.apple.WebKit.GPU\n40 1 90 com.apple.WebKit.WebContent\n50 30 20 /worker');
  assert.deepEqual(qa.ownedProcesses(rows, 10, new Map([[20, 10], [40, 99]])).map(row => row.pid), [10, 20, 30, 50]);
  assert.deepEqual(qa.ownedProcesses(rows, 10).map(row => row.pid), [10, 30, 50]);
  assert.equal(rows[0].command, '/bundle/HWP Workbench');
  assert.equal(qa.parseFootprint('phys_footprint: 123456 B\nphys_footprint_peak: 888888 B'), 123456);
  assert.equal(qa.parseFootprint('Footprint: 1.2 MiB'), null);
});

test('save evidence requires new text, changed hash and an exact backup of the original', () => {
  const evidence = {beforeHash: 'aaa', afterHash: 'bbb', beforeText: '기존', afterText: '한글',
    expectedText: '한글', backupHashes: ['aaa'], beforeFormat: 'hwpx', afterFormat: 'hwpx'};
  assert.deepEqual(qa.saveChecks(evidence), {newText: true, changedHash: true, originalBackup: true, sameFormat: true});
  assert.equal(qa.saveChecks({...evidence, afterHash: 'aaa'}).changedHash, false);
  assert.equal(qa.saveChecks({...evidence, backupHashes: ['ccc']}).originalBackup, false);
  assert.equal(qa.saveChecks({...evidence, beforeText: '한글'}).newText, false);
  assert.equal(qa.saveChecks({...evidence, afterFormat: 'hwp'}).sameFormat, false);
});

test('required native checks remain visible when a run stops before save', () => {
  const names = qa.nativeCheckNames(1);
  assert.ok(names.includes('save: originalBackup'));
  assert.ok(names.includes('control: Enter commits'));
  const omissions = qa.unexecutedChecks(names, [{name: 'on-screen window', passed: true}],
    [{check: 'control: Enter commits', required: true, reason: 'missing control'}]);
  assert.ok(omissions.some(item => item.check === 'save: originalBackup' && item.required));
  assert.equal(omissions.some(item => item.check === 'on-screen window'), false);
  assert.equal(omissions.some(item => item.check === 'control: Enter commits'), false);
});
