/** Build a self-contained Apple Silicon app and compressed DMG.
 * All build tools run as command-line processes. No application is launched.
 */
const fs = require('node:fs/promises');
const { createReadStream, constants } = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const runtimePin = require('./python-runtime.json');

const ROOT = path.resolve(__dirname, '..');
const APP_NAME = 'HWP Workbench';
const BUNDLE_ID = 'io.github.jonathanjie.hwp-workbench';
const APP_FILES = [
  'package.json',
  'desktop',
  'engine',
  'skills',
  'scripts/hwp-workbench',
  'fixtures/generated',
  'node_modules/@rhwp/core',
];

function run(command, args, options = {}) {
  return execFileSync(command, args, { stdio: 'inherit', ...options });
}

async function assertNewOutput(destination) {
  try {
    await fs.lstat(destination);
  } catch (error) {
    if (error.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  throw new Error(`Output exists; move it aside before building: ${destination}`);
}

async function verifyChecksum(file, expected) {
  const hash = crypto.createHash('sha256');
  for await (const chunk of createReadStream(file)) {
    hash.update(chunk);
  }
  const actual = hash.digest('hex');
  if (actual !== expected) {
    throw new Error(`Python runtime SHA-256 mismatch: ${file} (expected ${expected}, got ${actual})`);
  }
  return actual;
}

async function pinnedRuntime(environment = process.env) {
  const cache = environment.HWP_WORKBENCH_CACHE || path.join(os.homedir(), 'Library', 'Caches', 'hwp-workbench-build');
  const archive = path.join(cache, runtimePin.fileName);
  await fs.mkdir(cache, { recursive: true });
  try {
    await fs.lstat(archive);
  } catch (error) {
    if (error.code !== 'ENOENT') {
      throw error;
    }
    // Publish only verified bytes. A concurrent cache entry is never replaced.
    const temporary = await fs.mkdtemp(path.join(cache, '.python-download-'));
    try {
      const download = path.join(temporary, runtimePin.fileName);
      run('/usr/bin/curl', ['--fail', '--location', '--proto', '=https', '--max-time', '600', '--output', download, runtimePin.url]);
      await verifyChecksum(download, runtimePin.sha256);
      await fs.link(download, archive);
    } finally {
      await fs.rm(temporary, { recursive: true, force: true });
    }
  }
  await verifyChecksum(archive, runtimePin.sha256);
  return archive;
}

function shouldPruneRuntime(relativePath) {
  const parts = relativePath.split('/');
  const name = parts.at(-1);
  const unused = new Set(['test', 'tests', 'idlelib', 'tkinter', 'turtledemo', 'ensurepip', 'pip', 'lib2to3']);
  if (parts[0] === 'include' || parts.some(part => unused.has(part))) {
    return true;
  }
  if (parts.some(part => /^(?:pip-.*\.dist-info|tcl\d.*|tk\d.*|config-\d.*)$/.test(part))) {
    return true;
  }
  if (/^(?:libtcl|libtk)/.test(name) || /^(?:_tkinter|readline)\..*\.so$/.test(name)) {
    return true;
  }
  // Remove entry points for deleted tools and development configuration files.
  return parts[0] === 'bin' && /^(?:pip|idle|pydoc|python.*-config)/.test(name);
}

async function pruneRuntime(directory, relative = '') {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const member = relative ? `${relative}/${entry.name}` : entry.name;
    const target = path.join(directory, entry.name);
    if (shouldPruneRuntime(member)) {
      await fs.rm(target, { recursive: true, force: true });
    } else if (entry.isDirectory()) {
      await pruneRuntime(target, member);
    }
  }
}

function plistValues(version, minimumSystemVersion = '13.0') {
  // Apple bundle versions are numeric; retain the complete alpha version separately.
  const numericVersion = /^\d+\.\d+\.\d+/.exec(version)?.[0];
  if (!numericVersion) {
    throw new Error(`Invalid package version: ${version}`);
  }
  const types = [
    { extension: 'hwp', identifier: `${BUNDLE_ID}.hwp`, name: 'Hancom HWP document', mime: 'application/x-hwp', conforms: 'public.data' },
    { extension: 'hwpx', identifier: `${BUNDLE_ID}.hwpx`, name: 'Hancom HWPX document', mime: 'application/vnd.hancom.hwpx', conforms: 'public.zip-archive' },
  ];
  return {
    CFBundleName: APP_NAME,
    CFBundleDisplayName: APP_NAME,
    CFBundleExecutable: APP_NAME,
    CFBundleIdentifier: BUNDLE_ID,
    CFBundlePackageType: 'APPL',
    CFBundleShortVersionString: numericVersion,
    CFBundleVersion: numericVersion,
    HWPWorkbenchVersion: version,
    CFBundleIconFile: 'hwp-workbench.icns',
    LSMinimumSystemVersion: minimumSystemVersion,
    // Electron's application subclass and copied nib are required for native startup.
    NSPrincipalClass: 'AtomApplication',
    NSMainNibFile: 'MainMenu',
    CFBundleInfoDictionaryVersion: '6.0',
    LSApplicationCategoryType: 'public.app-category.productivity',
    NSRequiresAquaSystemAppearance: false,
    NSHighResolutionCapable: true,
    NSHumanReadableCopyright: 'Copyright © 2026 Jonathan Jie. Third-party notices are included in Resources.',
    CFBundleDocumentTypes: types.map(type => ({
      CFBundleTypeName: type.name,
      CFBundleTypeRole: 'Editor',
      LSHandlerRank: 'Alternate',
      CFBundleTypeExtensions: [type.extension],
      LSItemContentTypes: [type.identifier],
    })),
    UTImportedTypeDeclarations: types.map(type => ({
      UTTypeIdentifier: type.identifier,
      UTTypeDescription: type.name,
      UTTypeConformsTo: [type.conforms],
      UTTypeTagSpecification: {
        'public.filename-extension': [type.extension],
        'public.mime-type': type.mime,
      },
    })),
  };
}

function plistXML(metadata) {
  const escape = text => String(text).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
  function valueXML(value) {
    if (typeof value === 'boolean') {
      return value ? '<true/>' : '<false/>';
    }
    if (Array.isArray(value)) {
      return `<array>${value.map(valueXML).join('')}</array>`;
    }
    if (typeof value === 'object' && value !== null) {
      return `<dict>${Object.entries(value).map(([key, item]) => `<key>${escape(key)}</key>${valueXML(item)}`).join('')}</dict>`;
    }
    return `<string>${escape(value)}</string>`;
  }
  return `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">${valueXML(metadata)}</plist>\n`;
}

function includeAppFile(source) {
  const parts = path.relative(ROOT, source).split(path.sep);
  return !parts.some(part => ['test', 'tests', '__pycache__', '.cache', '.pytest_cache', '.DS_Store', 'cache', 'caches', 'coverage'].includes(part))
    && !/\.(?:test|spec)\.(?:cjs|mjs|js)$/.test(path.basename(source))
    && !source.endsWith('.pyc');
}

async function copyApp(resources, electronDirectory) {
  const appDirectory = path.join(resources, 'app');
  await fs.mkdir(appDirectory);
  for (const member of APP_FILES) {
    await fs.cp(path.join(ROOT, member), path.join(appDirectory, member), {
      recursive: true,
      dereference: false,
      verbatimSymlinks: true,
      filter: includeAppFile,
    });
  }
  // Preserve license texts supplied with Electron, Chromium and rhwp.
  const notices = path.join(resources, 'notices');
  await fs.mkdir(notices);
  for (const name of ['LICENSE', 'LICENSES.chromium.html']) {
    await fs.copyFile(path.join(electronDirectory, 'dist', name), path.join(notices, name));
  }
  await fs.copyFile(path.join(ROOT, 'THIRD_PARTY_NOTICES.md'), path.join(notices, 'THIRD_PARTY_NOTICES.md'));
  return appDirectory;
}

async function createIcon(iconSource, resources, workDirectory) {
  const iconset = path.join(workDirectory, 'hwp-workbench.iconset');
  await fs.mkdir(iconset);
  for (const points of [16, 32, 128, 256, 512]) {
    for (const scale of [1, 2]) {
      const pixels = points * scale;
      const name = `icon_${points}x${points}${scale === 2 ? '@2x' : ''}.png`;
      run('/usr/bin/sips', ['--resampleHeightWidth', String(pixels), String(pixels), iconSource, '--out', path.join(iconset, name)], { stdio: 'pipe' });
    }
  }
  run('/usr/bin/iconutil', ['--convert', 'icns', '--output', path.join(resources, 'hwp-workbench.icns'), iconset]);
}

async function prepareRuntime(archive, resources, appDirectory) {
  // The archive has a single python/ root. Its hash is checked immediately before extraction.
  await verifyChecksum(archive, runtimePin.sha256);
  run('/usr/bin/tar', ['-xzf', archive, '-C', resources]);
  const pythonDirectory = path.join(resources, 'python');
  await pruneRuntime(pythonDirectory);
  const python = path.join(pythonDirectory, 'bin', 'python3');
  const environment = { ...process.env, PYTHONUTF8: '1', PYTHONDONTWRITEBYTECODE: '1' };
  delete environment.PYTHONHOME;
  delete environment.PYTHONPATH;
  // Ignore developer environment/site packages; compilation occurs before signing.
  run(python, ['-I', '-m', 'compileall', '-q', path.join(appDirectory, 'engine'), path.join(pythonDirectory, 'lib', 'python3.13')], { env: environment });
  run(python, ['-I', path.join(ROOT, 'scripts', 'build-mcpb.py'), '--output', path.join(appDirectory, 'dist', 'hwp-workbench.mcpb')], { env: environment });
}

async function publishApp(source, target) {
  // mkdir is exclusive even if another build races after preflight. Keep a partial
  // owned output on copy failure so it cannot be mistaken for a successful rebuild.
  await fs.mkdir(target);
  await fs.cp(source, target, { recursive: true, force: false, errorOnExist: true, verbatimSymlinks: true });
}

async function directorySize(directory) {
  let size = 0;
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const member = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      size += await directorySize(member);
    } else {
      size += (await fs.lstat(member)).size;
    }
  }
  return size;
}

async function main() {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') {
    throw new Error('Run this build on Apple Silicon macOS with arm64 Node.js.');
  }
  const { version } = require(path.join(ROOT, 'package.json'));
  const outputDirectory = path.join(ROOT, 'outputs', 'mac-arm64');
  const target = path.join(outputDirectory, `${APP_NAME}.app`);
  const imageName = `HWP-Workbench-${version}-arm64.dmg`;
  const imageTarget = path.join(outputDirectory, imageName);
  await assertNewOutput(target);
  await assertNewOutput(imageTarget);
  const iconSource = path.join(ROOT, 'docs', 'design', 'proof', 'icon-1024.png');
  await fs.access(iconSource);
  const electronDirectory = path.dirname(require.resolve('electron/package.json'));
  const source = path.join(electronDirectory, 'dist', 'Electron.app');
  await fs.access(source);
  const minimumVersion = run('/usr/libexec/PlistBuddy', ['-c', 'Print :LSMinimumSystemVersion', path.join(source, 'Contents', 'Info.plist')], { stdio: 'pipe', encoding: 'utf8' }).trim();
  const archive = await pinnedRuntime();
  await fs.mkdir(path.join(ROOT, 'work'), { recursive: true });
  const workDirectory = await fs.mkdtemp(path.join(ROOT, 'work', 'package-mac-'));
  try {
    const volume = path.join(workDirectory, 'volume');
    await fs.mkdir(volume);
    const application = path.join(volume, `${APP_NAME}.app`);
    // Framework version links must remain relative after staging is removed.
    await fs.cp(source, application, { recursive: true, verbatimSymlinks: true });
    const contents = path.join(application, 'Contents');
    const resources = path.join(contents, 'Resources');
    await fs.rename(path.join(contents, 'MacOS', 'Electron'), path.join(contents, 'MacOS', APP_NAME));
    const appDirectory = await copyApp(resources, electronDirectory);
    await prepareRuntime(archive, resources, appDirectory);
    await createIcon(iconSource, resources, workDirectory);
    await fs.writeFile(path.join(contents, 'Info.plist'), plistXML(plistValues(version, minimumVersion)));
    run('/usr/bin/plutil', ['-lint', path.join(contents, 'Info.plist')]);
    await fs.rm(path.join(resources, 'default_app.asar'), { force: true });
    run('/usr/bin/codesign', ['--force', '--deep', '--sign', '-', application]);
    run('/usr/bin/codesign', ['--verify', '--deep', '--strict', application]);
    await fs.symlink('/Applications', path.join(volume, 'Applications'));
    const image = path.join(workDirectory, imageName);
    run('/usr/bin/hdiutil', ['create', '-volname', APP_NAME, '-srcfolder', volume, '-format', 'UDZO', image]);
    await fs.mkdir(outputDirectory, { recursive: true });
    await assertNewOutput(target);
    await assertNewOutput(imageTarget);
    await publishApp(application, target);
    await fs.copyFile(image, imageTarget, constants.COPYFILE_EXCL);
    console.log(`${target} (${await directorySize(target)} bytes)`);
    console.log(`${imageTarget} (${(await fs.stat(imageTarget)).size} bytes)`);
    console.log('Ad-hoc signed; not notarized.');
  } finally {
    await fs.rm(workDirectory, { recursive: true, force: true });
  }
}

module.exports = { verifyChecksum, pinnedRuntime, shouldPruneRuntime, pruneRuntime, plistValues, plistXML, assertNewOutput, includeAppFile, publishApp };

if (require.main === module) {
  main().catch(error => {
    console.error(`Package not built: ${error.message}`);
    process.exitCode = 1;
  });
}
