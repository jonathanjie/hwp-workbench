const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const {AgentFiles} = require('../agent-files.cjs');

const name = 'source.hwpx.20261009T120000Z-abcdef.json';
const nodeIds = ['native-a', 'native-b'];
function proposal(hash = 'a'.repeat(64)) {
  return {schemaVersion: 1, kind: 'hwp-workbench-proposal', document: {name: 'source.hwpx', sha256: hash},
    author: 'Claude', title: 'Review', createdAt: '2026-10-09T12:00:00Z',
    patch: {schemaVersion: 1, baseSha256: hash, operations: nodeIds.map(nodeId => ({op: 'set_text', nodeId, expectedText: 'old', text: 'new'}))}};
}

test('bounded reader distinguishes stale and unreadable proposals; outcomes are exclusive and complete', async () => {
  const root = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-link-')));
  const files = new AgentFiles(root, 'source.hwpx');
  const folder = path.join(root, '.hwp-workbench/proposals');
  try {
    await fs.mkdir(folder, {recursive: true});
    await fs.writeFile(path.join(folder, name), JSON.stringify(proposal()));
    let rows = await files.read('b'.repeat(64));
    assert.equal(rows[0].status, 'stale');
    rows = await files.read('a'.repeat(64));
    assert.equal(rows[0].status, 'pending');
    await assert.rejects(files.outcome(name, ['native-a'], []), /complete/i);
    await files.outcome(name, ['native-a'], ['native-b']);
    const outcome = JSON.parse(await fs.readFile(path.join(folder, name.replace(/\.json$/, '.outcome.json')), 'utf8'));
    assert.equal(outcome.status, 'partial');
    assert.deepEqual(outcome.acceptedNodeIds, ['native-a']);
    assert.deepEqual(outcome.declinedNodeIds, ['native-b']);
    await assert.rejects(files.outcome(name, nodeIds, []), /exist/i);
    await fs.writeFile(path.join(folder, name), Buffer.alloc(2 * 1024 * 1024 + 1));
    assert.equal((await files.read('a'.repeat(64)))[0].status, 'unreadable');
    await fs.rm(path.join(folder, name));
    await fs.writeFile(path.join(root, 'target.json'), JSON.stringify(proposal()));
    await fs.symlink(path.join(root, 'target.json'), path.join(folder, name));
    assert.equal((await files.read('a'.repeat(64)))[0].status, 'unreadable');
    await fs.rm(path.join(folder, name));
    const excessive = proposal();
    excessive.patch.operations = Array.from({length: 1001}, (_, index) => ({op: 'set_text', nodeId: `node-${index}`, expectedText: 'old', text: 'new'}));
    await fs.writeFile(path.join(folder, name), JSON.stringify(excessive));
    assert.equal((await files.read('a'.repeat(64)))[0].status, 'unreadable');
    const second = name.replace('abcdef', '123456');
    await fs.writeFile(path.join(folder, second), JSON.stringify(proposal()));
    const unsafeOutcome = path.join(folder, second.replace(/\.json$/, '.outcome.json'));
    await fs.symlink(path.join(root, 'target.json'), unsafeOutcome);
    // A readable proposal whose outcome cannot be trusted needs a fresh review.
    assert.equal((await files.read('a'.repeat(64))).find(row => row.file === second).status, 'needsReview');
    await assert.rejects(files.outcome(second, nodeIds, []), /exist|symlink/i);
    assert.deepEqual(JSON.parse(await fs.readFile(path.join(root, 'target.json'))), proposal());
    await fs.rm(path.join(root, '.hwp-workbench'), {recursive: true});
    await fs.symlink(root, path.join(root, '.hwp-workbench'));
    await assert.rejects(files.read('a'.repeat(64)), /symlink/i);
  } finally {
    await fs.rm(root, {recursive: true, force: true});
  }
});

test('atomic session writer refuses symlink destinations and proposal reader caps at 200', async () => {
  const root = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-link-')));
  const files = new AgentFiles(root, 'source.hwpx');
  try {
    const session = {schemaVersion: 1, app: 'hwp-workbench', appVersion: '0.1.0-alpha.1',
      document: {name: 'source.hwpx', format: 'hwpx', sha256: 'a'.repeat(64)}, selection: null, staged: null, updatedAt: '2026-10-09T12:00:00Z'};
    await files.session(session);
    await files.session({...session, selection: {nodeId: 'native-a', text: 'current'}});
    const destination = path.join(root, '.hwp-workbench/source.hwpx.session.json');
    assert.equal(JSON.parse(await fs.readFile(destination)).selection.text, 'current');
    await fs.rm(destination);
    await fs.writeFile(path.join(root, 'target'), 'untouched');
    await fs.symlink(path.join(root, 'target'), destination);
    await assert.rejects(files.session(session), /symlink/i);
    assert.equal(await fs.readFile(path.join(root, 'target'), 'utf8'), 'untouched');
    const folder = path.join(root, '.hwp-workbench/proposals');
    for (let i = 0; i < 201; i += 1) {
      await fs.writeFile(path.join(folder, `source.hwpx.20261009T120000Z-${i.toString(16).padStart(6, '0')}.json`), JSON.stringify(proposal()));
    }
    await fs.writeFile(path.join(folder, 'source.hwpx.other.20261009T120000Z-abcdef.json'), JSON.stringify(proposal()));
    assert.equal((await files.read('a'.repeat(64))).length, 200);
  } finally {
    await fs.rm(root, {recursive: true, force: true});
  }
});
