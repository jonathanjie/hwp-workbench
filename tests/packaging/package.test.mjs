import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

async function packaging() { return import('../../scripts/package-mac-tauri.mjs'); }
async function temporary(t) {
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'hwp-packaging-test-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  return root;
}
const sample = {
  productName: 'HWP & Workbench', version: '0.2.0-alpha.1', identifier: 'org.example.workbench',
  bundle: {macOS: {minimumSystemVersion: '12.0'}, fileAssociations: [
    {ext: ['hwp'], name: 'HWP document', role: 'Editor', rank: 'Alternate'},
    {ext: ['hwpx'], name: 'HWPX document', role: 'Viewer', rank: 'Default'}
  ]}
};
test('Info.plist uses config metadata, numeric versions, associations and native UTIs', async () => {
  const {plistValues, plistXML} = await packaging();
  const values = plistValues(sample);
  assert.equal(values.CFBundleName, sample.productName);
  assert.equal(values.CFBundleIdentifier, sample.identifier);
  assert.equal(values.CFBundleVersion, '0.2.0');
  assert.equal(values.HWPWorkbenchVersion, sample.version);
  assert.equal(values.LSMinimumSystemVersion, '12.0');
  assert.equal(values.NSHighResolutionCapable, true);
  assert.equal(values.CFBundleIconFile, 'icon.icns');
  assert.equal(values.CFBundleDocumentTypes[1].CFBundleTypeRole, 'Viewer');
  assert.equal(values.CFBundleDocumentTypes[1].LSHandlerRank, 'Default');
  assert.equal(values.UTImportedTypeDeclarations[1].UTTypeTagSpecification['public.mime-type'], 'application/vnd.hancom.hwpx');
  assert.match(plistXML(values), /HWP &amp; Workbench/);
});
for (const target of ['/absolute', '../escape', 'nested/../escape', 'C:\\absolute', '']) {
  test(`resource mapping refuses target ${JSON.stringify(target)}`, async t => {
    const {resourceMappings} = await packaging();
    const root = await temporary(t);
    await fs.writeFile(path.join(root, 'source'), 'safe');
    await assert.rejects(resourceMappings({'source': target}, root, root), /target/i);
  });
}
test('resource mapping reads the config list and copies nested regular files', async t => {
  const {resourceMappings, copyResources} = await packaging();
  const root = await temporary(t);
  await fs.mkdir(path.join(root, 'source/sub'), {recursive: true});
  await fs.writeFile(path.join(root, 'source/sub/file'), 'payload');
  const mappings = await resourceMappings({'source': 'app/data'}, root, root);
  await copyResources(mappings, path.join(root, 'output'));
  assert.equal(await fs.readFile(path.join(root, 'output/app/data/sub/file'), 'utf8'), 'payload');
});
for (const kind of ['missing', 'symlink', 'parent symlink', 'nested symlink']) {
  test(`resource mapping refuses ${kind}`, async t => {
    const {resourceMappings} = await packaging();
    const root = await temporary(t);
    await fs.mkdir(path.join(root, 'real'));
    await fs.writeFile(path.join(root, 'real/file'), 'payload');
    let source = 'missing';
    if (kind === 'symlink') { await fs.symlink('real/file', path.join(root, 'link')); source = 'link'; }
    if (kind === 'parent symlink') { await fs.symlink('real', path.join(root, 'link')); source = 'link/file'; }
    if (kind === 'nested symlink') { await fs.symlink('file', path.join(root, 'real/link')); source = 'real'; }
    await assert.rejects(resourceMappings({[source]: 'app/data'}, root, root));
  });
}
test('resource targets cannot overlap or replace generated bundle content', async t => {
  const {resourceMappings} = await packaging();
  const root = await temporary(t);
  await fs.writeFile(path.join(root, 'source'), 'safe');
  await assert.rejects(resourceMappings({'source': 'icon.icns'}, root, root), /reserved/i);
});
test('preflight refuses existing outputs including dangling symlinks', async t => {
  const {assertNewOutput} = await packaging();
  const root = await temporary(t);
  await fs.symlink('missing', path.join(root, 'output'));
  await assert.rejects(assertNewOutput(path.join(root, 'output')), /exists/i);
});
test('exclusive publication preserves outputs created after preflight', async t => {
  const {publishApp, assertNewOutput} = await packaging();
  const root = await temporary(t);
  const source = path.join(root, 'source');
  const output = path.join(root, 'output');
  await fs.mkdir(source); await fs.writeFile(path.join(source, 'file'), 'new');
  await assertNewOutput(output);
  await fs.mkdir(output); await fs.writeFile(path.join(output, 'file'), 'user work');
  await assert.rejects(publishApp(source, output));
  assert.equal(await fs.readFile(path.join(output, 'file'), 'utf8'), 'user work');
});
test('CLI symlink is relative and resolves to the app executable', async t => {
  const {createCliLink, checkBundleContent} = await packaging();
  const root = await temporary(t);
  await fs.mkdir(path.join(root, 'Contents/MacOS'), {recursive: true});
  await fs.writeFile(path.join(root, 'Contents/MacOS/HWP Workbench'), 'binary');
  await createCliLink(root, 'HWP Workbench');
  const link = path.join(root, 'Contents/Resources/bin/hwp-workbench');
  assert.equal(await fs.readlink(link), '../../MacOS/HWP Workbench');
  assert.equal(await fs.realpath(link), await fs.realpath(path.join(root, 'Contents/MacOS/HWP Workbench')));
  await checkBundleContent(root, 'HWP Workbench');
});
for (const member of ['Contents/Resources/engine.py', 'Contents/Resources/node_modules/file', 'Contents/Frameworks/Unexpected Framework.framework/file']) {
  test(`content check refuses ${member}`, async t => {
    const {checkBundleContent} = await packaging();
    const root = await temporary(t);
    const file = path.join(root, member);
    await fs.mkdir(path.dirname(file), {recursive: true}); await fs.writeFile(file, 'unsafe');
    await assert.rejects(checkBundleContent(root, 'HWP Workbench'), /forbidden/i);
  });
}
test('resource mapping rejects overlapping file destinations', async t => {
  const {resourceMappings} = await packaging();
  const root = await temporary(t);
  await fs.writeFile(path.join(root, 'one'), 'first');
  await fs.writeFile(path.join(root, 'two'), 'second');
  await assert.rejects(resourceMappings({one: 'app/file', two: 'app/file/child'}, root, root), /overlap/i);
});
test('resources that become symlinks before copying are refused', async t => {
  const {resourceMappings, copyResources} = await packaging();
  const root = await temporary(t);
  await fs.writeFile(path.join(root, 'source'), 'first');
  await fs.writeFile(path.join(root, 'other'), 'second');
  const mappings = await resourceMappings({source: 'app/file'}, root, root);
  await fs.rm(path.join(root, 'source'));
  await fs.symlink('other', path.join(root, 'source'));
  await assert.rejects(copyResources(mappings, path.join(root, 'output')), /symlink/i);
});
test('bundle content check refuses arbitrary relative symlinks', async t => {
  const {checkBundleContent} = await packaging();
  const root = await temporary(t);
  await fs.symlink('../../etc/passwd', path.join(root, 'link'));
  await assert.rejects(checkBundleContent(root, 'HWP Workbench'), /unexpected symlink/i);
});
for (const version of ['0.2.0-alpha/../../escape', '0.2.0-alpha 1', '0.2.0-']) {
  test(`bundle metadata rejects invalid or path-bearing version ${version}`, async () => {
    const {plistValues} = await packaging();
    assert.throws(() => plistValues({...sample, version}), /version/i);
  });
}
test('bundle metadata rejects a nonnumeric CFBundleVersion override', async () => {
  const {plistValues} = await packaging();
  assert.throws(() => plistValues({...sample, bundle: {...sample.bundle, macOS: {bundleVersion: 'alpha'}}}), /version/i);
});
test('overlap detection also rejects ancestors separated by sorted siblings', async t => {
  const {resourceMappings} = await packaging();
  const root = await temporary(t);
  for (const file of ['one', 'two', 'three']) await fs.writeFile(path.join(root, file), file);
  await assert.rejects(resourceMappings({one: 'a', two: 'a-b', three: 'a/c'}, root, root), /overlap/i);
});
