/** Assemble and verify an offline Tauri bundle without launching any GUI. */
import fs from 'node:fs/promises';
import {constants, createReadStream} from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {cargoCommand, cargoMetadata, generateNotices} from './third-party-notices.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const configRoot = path.join(root, 'app/src-tauri');
const MB = 1_000_000; // Decimal MB, matching the size budgets in the rewrite plan.
function run(command, args, options = {}) {
  return execFileSync(command, args, {cwd: root, stdio: ['ignore', 'inherit', 'inherit'], ...options});
}
export function plistValues(config) {
  const version = /^\d+\.\d+\.\d+(?:-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?(?:\+[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?$/.exec(config.version)?.[0];
  if (!version) throw new Error(`Invalid package version: ${config.version}`);
  const numericVersion = version.split(/[-+]/)[0];
  const bundleVersion = config.bundle?.macOS?.bundleVersion || numericVersion;
  if (!/^\d+(?:\.\d+){0,2}$/.test(bundleVersion)) throw new Error(`Invalid numeric bundle version: ${bundleVersion}`);
  if (!config.identifier || !config.productName || /[\/\\\0]/.test(config.productName)) {
    throw new Error('Invalid bundle identifier or product name');
  }
  const nativeTypes = [
    {ext: 'hwp', name: 'Hancom HWP document', mime: 'application/x-hwp', conforms: 'public.data'},
    {ext: 'hwpx', name: 'Hancom HWPX document', mime: 'application/vnd.hancom.hwpx', conforms: 'public.zip-archive'}
  ];
  const values = {
    CFBundleName: config.bundle?.macOS?.bundleName || config.productName,
    CFBundleDisplayName: config.productName, CFBundleExecutable: config.productName,
    CFBundleIdentifier: config.identifier, CFBundlePackageType: 'APPL',
    CFBundleShortVersionString: numericVersion, CFBundleVersion: bundleVersion,
    HWPWorkbenchVersion: version, CFBundleIconFile: 'icon.icns', CFBundleInfoDictionaryVersion: '6.0',
    NSPrincipalClass: 'NSApplication', NSHighResolutionCapable: true,
    LSApplicationCategoryType: 'public.app-category.productivity',
    CFBundleDocumentTypes: (config.bundle?.fileAssociations || []).map(type => ({
      CFBundleTypeName: type.name || type.ext[0], CFBundleTypeRole: type.role || 'Editor',
      LSHandlerRank: type.rank || 'Default', CFBundleTypeExtensions: type.ext,
      LSItemContentTypes: type.ext.map(ext => `${config.identifier}.${ext}`)
    })),
    UTImportedTypeDeclarations: nativeTypes.map(type => ({
      UTTypeIdentifier: `${config.identifier}.${type.ext}`, UTTypeDescription: type.name,
      UTTypeConformsTo: [type.conforms], UTTypeTagSpecification: {
        'public.filename-extension': [type.ext], 'public.mime-type': type.mime
      }
    }))
  };
  const minimumVersion = config.bundle?.macOS?.minimumSystemVersion;
  if (minimumVersion !== null) values.LSMinimumSystemVersion = minimumVersion || '10.13';
  return values;
}
export function plistXML(metadata) {
  const escape = text => String(text).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
  function valueXML(value) {
    if (typeof value === 'boolean') return value ? '<true/>' : '<false/>';
    if (Array.isArray(value)) return `<array>${value.map(valueXML).join('')}</array>`;
    if (value && typeof value === 'object') return `<dict>${Object.entries(value).map(([key, item]) => `<key>${escape(key)}</key>${valueXML(item)}`).join('')}</dict>`;
    return `<string>${escape(value)}</string>`;
  }
  return `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">${valueXML(metadata)}</plist>\n`;
}
export async function assertNewOutput(destination) {
  try { await fs.lstat(destination); }
  catch (error) { if (error.code === 'ENOENT') return; throw error; }
  throw new Error(`Output exists; move it aside before building: ${destination}`);
}
async function noSymlinkParents(file) {
  const absolute = path.resolve(file);
  const parts = absolute.split(path.sep).filter(Boolean);
  let current = path.parse(absolute).root;
  for (const part of parts) {
    current = path.join(current, part);
    try {
      if ((await fs.lstat(current)).isSymbolicLink()) throw new Error(`Refuse symlink: ${current}`);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
}
async function regularTree(source, target, files) {
  const stat = await fs.lstat(source);
  if (stat.isSymbolicLink()) throw new Error(`Refuse resource symlink: ${source}`);
  if (stat.isFile()) { files.push({source, target}); return; }
  if (!stat.isDirectory()) throw new Error(`Resource is not a regular file or directory: ${source}`);
  for (const name of (await fs.readdir(source)).sort()) await regularTree(path.join(source, name), `${target}/${name}`, files);
}
function safeTarget(target) {
  if (typeof target !== 'string' || !target || path.posix.isAbsolute(target) || path.win32.isAbsolute(target)
    || target.includes('\\') || target.includes('\0') || target.split('/').some(part => ['..', '.', ''].includes(part))) {
    throw new Error(`Invalid resource target: ${target}`);
  }
  if (['icon.icns', 'THIRD_PARTY_NOTICES.txt', 'bin'].some(reserved => target === reserved || target.startsWith(`${reserved}/`))) {
    throw new Error(`Reserved resource target: ${target}`);
  }
}
export async function resourceMappings(resources, base = configRoot, project = root) {
  if (!resources) return [];
  const entries = Array.isArray(resources) ? resources.map(source => [source, source]) : Object.entries(resources);
  const files = [];
  for (const [source, target] of entries) {
    safeTarget(target);
    const absolute = path.resolve(base, source);
    const relative = path.relative(project, absolute);
    if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error(`Resource outside project: ${source}`);
    await noSymlinkParents(absolute);
    await regularTree(absolute, target, files);
  }
  const targets = new Set();
  for (const file of files) {
    if (targets.has(file.target)) throw new Error(`Overlapping resource target: ${file.target}`);
    targets.add(file.target);
  }
  for (const target of targets) {
    let parent = path.posix.dirname(target);
    while (parent !== '.') {
      if (targets.has(parent)) throw new Error(`Overlapping resource target: ${target}`);
      parent = path.posix.dirname(parent);
    }
  }
  return files.sort((a, b) => a.target < b.target ? -1 : a.target > b.target ? 1 : 0);
}
export async function copyResources(mappings, resources) {
  for (const file of mappings) {
    await noSymlinkParents(file.source);
    const destination = path.join(resources, file.target);
    await fs.mkdir(path.dirname(destination), {recursive: true});
    await fs.copyFile(file.source, destination, constants.COPYFILE_EXCL);
  }
}
export async function createCliLink(application, productName) {
  const directory = path.join(application, 'Contents/Resources/bin');
  await fs.mkdir(directory, {recursive: true});
  await fs.symlink(`../../MacOS/${productName}`, path.join(directory, 'hwp-workbench'));
}
export async function checkBundleContent(application, productName) {
  const forbidden = [];
  async function scan(directory) {
    for (const entry of await fs.readdir(directory, {withFileTypes: true})) {
      const file = path.join(directory, entry.name);
      const relative = path.relative(application, file).split(path.sep).join('/');
      if (/\.py$/i.test(entry.name) || entry.name.toLowerCase() === 'node_modules' || relative.startsWith('Contents/Frameworks/')) forbidden.push(relative);
      if (entry.isSymbolicLink()) {
        const allowed = relative === 'Contents/Resources/bin/hwp-workbench' && await fs.readlink(file) === `../../MacOS/${productName}`;
        if (!allowed) forbidden.push(`${relative} (unexpected symlink)`);
      } else if (entry.isDirectory()) await scan(file);
      else if (!entry.isFile()) forbidden.push(`${relative} (non-regular file)`);
    }
  }
  await scan(application);
  if (forbidden.length) throw new Error(`Forbidden bundle content:\n${forbidden.sort().join('\n')}`);
  return {passed: true, forbidden: []};
}
export async function publishApp(source, target) {
  await noSymlinkParents(path.dirname(target));
  // mkdir is exclusive. On failure keep the owned partial output for diagnosis.
  await fs.mkdir(target);
  await fs.cp(source, target, {recursive: true, force: false, errorOnExist: true, verbatimSymlinks: true});
}
async function sha256(file) {
  const hash = crypto.createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}
async function bundleMetrics(application) {
  const hash = crypto.createHash('sha256');
  let bytes = 0;
  // The app digest walks sorted directory entries and includes file/link metadata and bytes.
  async function visit(directory) {
    for (const name of (await fs.readdir(directory)).sort()) {
      const file = path.join(directory, name);
      const stat = await fs.lstat(file);
      const relative = path.relative(application, file).split(path.sep).join('/');
      if (stat.isDirectory()) { await visit(file); continue; }
      bytes += stat.size;
      hash.update(`${stat.isSymbolicLink() ? 'link' : 'file'}\0${relative}\0${stat.size}\0`);
      if (stat.isSymbolicLink()) hash.update(await fs.readlink(file));
      else for await (const chunk of createReadStream(file)) hash.update(chunk);
      hash.update('\0');
    }
  }
  await visit(application);
  return {bytes, sizeMB: bytes / MB, sha256: hash.digest('hex')};
}
function options(args) {
  const result = {config: path.join(configRoot, 'tauri.conf.json'), identity: '-'};
  for (let i = 0; i < args.length; i += 2) {
    if (!['--config', '--identity'].includes(args[i]) || !args[i + 1] || args[i + 1].startsWith('--')) throw new Error('Usage: package-mac-tauri.mjs [--config <path>] [--identity <name>]');
    result[args[i].slice(2)] = args[i + 1];
  }
  return result;
}
export async function packageMac(args = process.argv.slice(2)) {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') throw new Error('Run on Apple Silicon macOS with arm64 Node.js.');
  const selected = options(args);
  const config = JSON.parse(await fs.readFile(selected.config, 'utf8'));
  const plist = plistValues(config);
  const output = path.join(root, 'dist-mac');
  const appName = `${config.productName}.app`;
  const imageName = `HWP-Workbench-${config.version}-arm64.dmg`;
  const appTarget = path.join(output, appName);
  const imageTarget = path.join(output, imageName);
  const targets = [appTarget, imageTarget, `${appTarget}.sha256`, `${imageTarget}.sha256`];
  await noSymlinkParents(output);
  for (const target of targets) await assertNewOutput(target);
  // Config sources are always relative to the Tauri crate, including temporary config copies.
  await resourceMappings(config.bundle?.resources);
  run('npm', ['run', 'build:frontend']);
  run(cargoCommand(), ['build', '--offline', '--locked', '--release', '-p', 'hwp-workbench-app']);
  const metadata = cargoMetadata();
  const binary = path.join(metadata.target_directory, 'release', 'hwp-workbench-app');
  const notices = await generateNotices(metadata);
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-package-mac-'));
  try {
    const volume = path.join(temporary, 'volume');
    const application = path.join(volume, appName);
    const contents = path.join(application, 'Contents');
    const resources = path.join(contents, 'Resources');
    await fs.mkdir(path.join(contents, 'MacOS'), {recursive: true});
    await fs.mkdir(resources);
    const bundledBinary = path.join(contents, 'MacOS', config.productName);
    await fs.copyFile(binary, bundledBinary, constants.COPYFILE_EXCL);
    await fs.chmod(bundledBinary, 0o755);
    const icon = path.join(configRoot, 'icons/icon.icns');
    await noSymlinkParents(icon);
    await fs.copyFile(icon, path.join(resources, 'icon.icns'), constants.COPYFILE_EXCL);
    // Reread the resource mappings after Cargo, which can generate source resources.
    await copyResources(await resourceMappings(config.bundle?.resources), resources);
    await fs.writeFile(path.join(resources, 'THIRD_PARTY_NOTICES.txt'), notices.text, {flag: 'wx'});
    await fs.writeFile(path.join(contents, 'Info.plist'), plistXML(plist), {flag: 'wx'});
    await createCliLink(application, config.productName);
    const contentCheck = await checkBundleContent(application, config.productName);
    run('/usr/bin/plutil', ['-lint', path.join(contents, 'Info.plist')]);
    const signing = ['--force', '--sign', selected.identity];
    // Ad-hoc signing must not enable the hardened runtime. Developer ID is untested.
    if (selected.identity !== '-') signing.push('--options', 'runtime', '--timestamp');
    run('/usr/bin/codesign', [...signing, bundledBinary]);
    run('/usr/bin/codesign', [...signing, application]);
    run('/usr/bin/codesign', ['--verify', '--strict', '--deep', application]);
    await fs.symlink('/Applications', path.join(volume, 'Applications'));
    const formats = {};
    for (const format of ['UDZO', 'ULFO']) {
      const image = path.join(temporary, `${format}.dmg`);
      run('/usr/bin/hdiutil', ['create', '-volname', config.productName, '-srcfolder', volume, '-format', format, image]);
      run('/usr/bin/hdiutil', ['verify', image]);
      formats[format] = {path: image, bytes: (await fs.stat(image)).size};
    }
    const format = formats.ULFO.bytes < formats.UDZO.bytes ? 'ULFO' : 'UDZO';
    const image = formats[format];
    const app = await bundleMetrics(application);
    const dmg = {bytes: image.bytes, sizeMB: image.bytes / MB, sha256: await sha256(image.path)};
    const executable = {bytes: (await fs.stat(bundledBinary)).size, sha256: await sha256(bundledBinary)};
    executable.sizeMB = executable.bytes / MB;
    await fs.mkdir(output, {recursive: true});
    for (const target of targets) await assertNewOutput(target);
    await publishApp(application, appTarget);
    await fs.copyFile(image.path, imageTarget, constants.COPYFILE_EXCL);
    const publishedApp = await bundleMetrics(appTarget);
    if (publishedApp.sha256 !== app.sha256 || await sha256(imageTarget) !== dmg.sha256) {
      throw new Error('Published package checksum does not match the verified staging copy');
    }
    await fs.writeFile(`${appTarget}.sha256`, `${app.sha256}  ${appName}\n`, {flag: 'wx'});
    await fs.writeFile(`${imageTarget}.sha256`, `${dmg.sha256}  ${imageName}\n`, {flag: 'wx'});
    run('/usr/bin/codesign', ['--verify', '--strict', '--deep', appTarget]);
    await checkBundleContent(appTarget, config.productName);
    const summary = {
      host: {platform: process.platform, arch: process.arch, release: os.release()}, config: path.resolve(selected.config),
      paths: {app: appTarget, dmg: imageTarget, binary: path.join(appTarget, 'Contents/MacOS', config.productName)},
      binary: executable, app, dmg, format,
      dmgFormats: Object.fromEntries(Object.entries(formats).map(([name, value]) => [name, {bytes: value.bytes, sizeMB: value.bytes / MB}])),
      budgets: {app: {targetMB: 60, passed: app.sizeMB <= 60}, dmg: {targetMB: 30, passed: dmg.sizeMB <= 30}},
      contentCheck, licensePackages: notices.packages.length, signing: selected.identity === '-' ? 'ad-hoc; not notarized' : 'Developer ID; not notarized',
      appDigest: 'SHA-256 in depth-first order of sorted directory entries: file/link type, relative path, lstat size and content, NUL-delimited; not a shasum-compatible directory digest'
    };
    console.log(JSON.stringify(summary, null, 2));
    return summary;
  } finally { await fs.rm(temporary, {recursive: true, force: true}); }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  packageMac().catch(error => {console.error(`Package not built: ${error.message}`); process.exitCode = 1;});
}
