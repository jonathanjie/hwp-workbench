const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const runtime = require('../runtime.cjs');

const resourcesPath = '/Applications/HWP Workbench.app/Contents/Resources';
const packaged = { app: { isPackaged: true } };
const development = { app: { isPackaged: false } };
const settings = { resourcesPath, environment: {}, execPath: '/Applications/HWP Workbench.app/Contents/MacOS/HWP Workbench' };

test('packaged and development paths keep spaces and use the correct root', () => {
  assert.equal(runtime.appRoot(packaged, settings), `${resourcesPath}/app`);
  assert.equal(runtime.appRoot(development), path.resolve(__dirname, '..', '..'));
  assert.equal(runtime.cliPath(packaged, settings), `${resourcesPath}/app/scripts/hwp-workbench`);
  assert.equal(runtime.bundlePath(packaged, settings), `${resourcesPath}/app/dist/hwp-workbench.mcpb`);
});

test('packaged Python is bundled; an explicit override takes precedence', () => {
  assert.equal(runtime.pythonCommand(packaged, settings), `${resourcesPath}/python/bin/python3`);
  assert.equal(runtime.pythonCommand(packaged, { ...settings, environment: { HWP_WORKBENCH_PYTHON: '/custom/python' } }), '/custom/python');
});

test('development skips Python 3.9, failures, malformed output and timed-out probes', () => {
  const responses = [
    { status: 0, stdout: '3 9\n' },
    { status: 1, stdout: '' },
    { status: 0, stdout: 'not a version' },
    { status: null, error: new Error('timeout') },
    { status: 0, stdout: '3 13\n' },
  ];
  const calls = [];
  const spawnSync = (command, args, options) => {
    calls.push(command);
    assert.equal(options.shell, false);
    assert.equal(options.timeout, 10000);
    assert.equal(options.maxBuffer, 4096);
    return responses.shift();
  };
  assert.equal(runtime.pythonCommand(development, { environment: {}, spawnSync, home: '/test/home' }), '/Library/Frameworks/Python.framework/Versions/Current/bin/python3');
  assert.equal(calls.length, 5);
});

test('Python 3.11 is accepted; unavailable interpreters return null', () => {
  assert.equal(runtime.pythonCommand(development, { environment: {}, spawnSync: () => ({ status: 0, stdout: '3 11' }) }), '/opt/homebrew/bin/python3');
  assert.equal(runtime.pythonCommand(development, { environment: {}, spawnSync: () => ({ status: 0, stdout: '3 9' }) }), null);
});

test('engine environment is a copy and enables Electron as the Node bridge', () => {
  const environment = { PATH: '/usr/bin', PYTHONPATH: '/untrusted', PYTHONUTF8: '0' };
  const result = runtime.engineEnvironment(packaged, { ...settings, environment });
  assert.equal(result.PYTHONPATH, `${resourcesPath}/app/engine`);
  assert.equal(result.PYTHONUTF8, '1');
  assert.equal(result.PYTHONDONTWRITEBYTECODE, '1');
  assert.equal(result.HWP_WORKBENCH_NODE, settings.execPath);
  assert.equal(result.ELECTRON_RUN_AS_NODE, '1');
  assert.equal(environment.PYTHONUTF8, '0');
  assert.equal(environment.HWP_WORKBENCH_NODE, undefined);
});

test('an explicitly selected Node bridge is retained', () => {
  const result = runtime.engineEnvironment(development, { environment: { HWP_WORKBENCH_NODE: '/custom/node' }, execPath: '/electron' });
  assert.equal(result.HWP_WORKBENCH_NODE, '/custom/node');
  assert.equal(result.ELECTRON_RUN_AS_NODE, undefined);
});

// Packager helpers are imported without starting a build.
const fs = require('node:fs/promises');
const os = require('node:os');
const crypto = require('node:crypto');
const packaging = require('../../scripts/package-mac.cjs');

async function scratch(run) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-package-test-'));
  try {
    await run(directory);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
}

test('checksum verification accepts pinned bytes and refuses altered bytes', async () => {
  await scratch(async directory => {
    const archive = path.join(directory, 'runtime.tar.gz');
    const bytes = Buffer.from('runtime bytes');
    await fs.writeFile(archive, bytes);
    const expected = crypto.createHash('sha256').update(bytes).digest('hex');
    assert.equal(await packaging.verifyChecksum(archive, expected), expected);
    await fs.appendFile(archive, 'changed');
    await assert.rejects(packaging.verifyChecksum(archive, expected), /SHA-256 mismatch/);
  });
});

test('prune policy removes development and GUI components but keeps engine libraries and license', () => {
  for (const name of ['include/python3.13/Python.h', 'lib/python3.13/test', 'lib/python3.13/idlelib', 'lib/python3.13/tkinter', 'lib/python3.13/turtledemo', 'lib/python3.13/ensurepip', 'lib/python3.13/site-packages/pip', 'lib/python3.13/site-packages/pip-26.2.1.dist-info', 'lib/python3.13/lib2to3', 'lib/libtcl9.0.dylib', 'lib/libtcl9tk9.0.dylib', 'lib/tcl9.0', 'lib/python3.13/lib-dynload/readline.cpython-313-darwin.so', 'lib/python3.13/lib-dynload/_tkinter.cpython-313-darwin.so', 'bin/pip3.13', 'lib/python3.13/config-3.13-darwin']) {
    assert.equal(packaging.shouldPruneRuntime(name), true, name);
  }
  for (const name of ['lib/python3.13/LICENSE.txt', 'lib/libpython3.13.dylib', 'lib/python3.13/ctypes', 'lib/python3.13/sqlite3', 'lib/python3.13/ssl.py', 'lib/python3.13/lib-dynload/_decimal.cpython-313-darwin.so', 'bin/python3']) {
    assert.equal(packaging.shouldPruneRuntime(name), false, name);
  }
});

test('plist identifies the version, executable, HWP/HWPX editor roles and imported UTIs', () => {
  const metadata = packaging.plistValues('0.1.0-alpha.1', '12.0');
  assert.equal(metadata.CFBundleExecutable, 'HWP Workbench');
  assert.equal(metadata.NSPrincipalClass, 'AtomApplication');
  assert.equal(metadata.NSMainNibFile, 'MainMenu');
  assert.equal(metadata.CFBundleIdentifier, 'io.github.jonathanjie.hwp-workbench');
  assert.equal(metadata.CFBundleShortVersionString, '0.1.0');
  assert.equal(metadata.CFBundleVersion, '0.1.0');
  assert.equal(metadata.HWPWorkbenchVersion, '0.1.0-alpha.1');
  assert.equal(metadata.LSMinimumSystemVersion, '12.0');
  assert.equal(metadata.CFBundleIconFile, 'hwp-workbench.icns');
  assert.equal(metadata.CFBundleDocumentTypes.length, 2);
  for (const document of metadata.CFBundleDocumentTypes) {
    assert.equal(document.CFBundleTypeRole, 'Editor');
    assert.equal(document.LSHandlerRank, 'Alternate');
  }
  assert.deepEqual(metadata.UTImportedTypeDeclarations.map(type => type.UTTypeTagSpecification['public.filename-extension']), [['hwp'], ['hwpx']]);
  const xml = packaging.plistXML({ ...metadata, NSHumanReadableCopyright: 'A & B <C>' });
  assert.match(xml, /A &amp; B &lt;C&gt;/);
  assert.match(xml, /<array>/);
});

test('output refusal preserves existing files, directories and dangling symlinks', async () => {
  await scratch(async directory => {
    const output = path.join(directory, 'output');
    await packaging.assertNewOutput(output);
    await fs.writeFile(output, 'keep');
    await assert.rejects(packaging.assertNewOutput(output), /exists/);
    assert.equal(await fs.readFile(output, 'utf8'), 'keep');
    await fs.unlink(output);
    await fs.mkdir(output);
    await assert.rejects(packaging.assertNewOutput(output), /exists/);
    await fs.rmdir(output);
    await fs.symlink(path.join(directory, 'missing'), output);
    await assert.rejects(packaging.assertNewOutput(output), /exists/);
  });
});

const { execFileSync } = require('node:child_process');

test('CLI prefers bundled Python, then override, then PATH and supplies bundled Node', async () => {
  await scratch(async directory => {
    const contents = path.join(directory, 'Applications', 'HWP Workbench.app', 'Contents');
    const root = path.join(contents, 'Resources', 'app');
    const script = path.join(root, 'scripts', 'hwp-workbench');
    const bundledPython = path.join(contents, 'Resources', 'python', 'bin', 'python3');
    const executable = path.join(contents, 'MacOS', 'HWP Workbench');
    const override = path.join(directory, 'override-python');
    const pathPython = path.join(directory, 'bin', 'python3');
    for (const file of [script, bundledPython, executable, override, pathPython]) {
      await fs.mkdir(path.dirname(file), { recursive: true });
    }
    await fs.copyFile(path.join(__dirname, '..', '..', 'scripts', 'hwp-workbench'), script);
    for (const [file, label] of [[bundledPython, 'bundled'], [override, 'override'], [pathPython, 'path']]) {
      await fs.writeFile(file, `#!/bin/bash\nprintf '%s\\n' '${label}' "$HWP_WORKBENCH_NODE" "$ELECTRON_RUN_AS_NODE" "$PYTHONDONTWRITEBYTECODE" "$@"\n`, { mode: 0o755 });
    }
    await fs.writeFile(executable, '#!/bin/bash\nexit 0\n', { mode: 0o755 });
    const environment = { ...process.env, PATH: `${path.dirname(pathPython)}:/usr/bin:/bin`, HWP_WORKBENCH_PYTHON: override };
    delete environment.HWP_WORKBENCH_NODE;
    delete environment.ELECTRON_RUN_AS_NODE;
    const invoke = env => execFileSync('/bin/bash', [script, 'mcp', '--root', directory], { env, encoding: 'utf8' }).trim().split('\n');
    const output = invoke(environment);
    assert.equal(output[0], 'bundled');
    assert.equal(output[1], executable);
    assert.equal(output[2], '1');
    assert.equal(output[3], '1');
    assert.deepEqual(output.slice(4), ['-m', 'hwp_workbench', 'mcp', '--root', directory]);
    assert.equal(invoke({ ...environment, HWP_WORKBENCH_NODE: '/custom/node', ELECTRON_RUN_AS_NODE: '0' })[1], '/custom/node');
    await fs.unlink(bundledPython);
    assert.equal(invoke(environment)[0], 'override');
    delete environment.HWP_WORKBENCH_PYTHON;
    assert.equal(invoke(environment)[0], 'path');
  });
});

test('a corrupt cached runtime is refused without replacing it', async () => {
  await scratch(async directory => {
    const pin = require('../../scripts/python-runtime.json');
    const archive = path.join(directory, pin.fileName);
    await fs.writeFile(archive, 'corrupt cached archive');
    await assert.rejects(packaging.pinnedRuntime({ HWP_WORKBENCH_CACHE: directory }), /SHA-256 mismatch/);
    assert.equal(await fs.readFile(archive, 'utf8'), 'corrupt cached archive');
  });
});

test('app copy excludes tests and caches while retaining bridge, skill, rhwp and generated fixtures', () => {
  const root = path.resolve(__dirname, '..', '..');
  for (const member of ['desktop/runtime.test.cjs', 'engine/hwp_workbench/__pycache__/cli.pyc', 'desktop/.cache/data', 'node_modules/@rhwp/core/tests/example.hwp']) {
    assert.equal(packaging.includeAppFile(path.join(root, member)), false, member);
  }
  for (const member of ['desktop/runtime.cjs', 'engine/bridge/rhwp-bridge.mjs', 'skills/hwp-workbench/SKILL.md', 'node_modules/@rhwp/core/rhwp_bg.wasm', 'node_modules/@rhwp/core/LICENSE', 'fixtures/generated/review-form.hwpx']) {
    assert.equal(packaging.includeAppFile(path.join(root, member)), true, member);
  }
});

test('publishing an app keeps framework symlinks relative and refuses an existing app', async () => {
  await scratch(async directory => {
    const source = path.join(directory, 'staging', 'HWP Workbench.app');
    const target = path.join(directory, 'HWP Workbench.app');
    const versions = path.join(source, 'Contents', 'Frameworks', 'Example.framework', 'Versions');
    await fs.mkdir(path.join(versions, 'A'), { recursive: true });
    await fs.writeFile(path.join(versions, 'A', 'Example'), 'framework bytes');
    await fs.symlink('A', path.join(versions, 'Current'));
    await packaging.publishApp(source, target);
    const publishedVersions = path.join(target, 'Contents', 'Frameworks', 'Example.framework', 'Versions');
    assert.equal(await fs.readlink(path.join(publishedVersions, 'Current')), 'A');
    await fs.rm(source, { recursive: true });
    assert.equal(await fs.readFile(path.join(publishedVersions, 'Current', 'Example'), 'utf8'), 'framework bytes');
    await assert.rejects(packaging.publishApp(source, target), { code: 'EEXIST' });
  });
});
