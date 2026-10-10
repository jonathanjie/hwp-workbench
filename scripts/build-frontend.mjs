import fs from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const projectRoot = fileURLToPath(new URL('../', import.meta.url));
// These extensions are the MIME allowlist in desktop/protocol.cjs. Only
// renderer roots are scanned: backend code and tests are never bundled.
const rendererExtensions = new Set(['.html', '.css', '.js', '.svg', '.png', '.wasm']);

async function rendererFiles(root, relative = '', recursive = false) {
  const files = [];
  const entries = await fs.readdir(path.join(root, relative), {withFileTypes: true});
  for (const entry of entries) {
    const name = path.join(relative, entry.name);
    if (entry.isDirectory()) {
      if (recursive || (!relative && ['assets', 'styles'].includes(entry.name))) {
        files.push(...await rendererFiles(root, name, true));
      }
      continue;
    }
    if (!rendererExtensions.has(path.extname(name))) {
      continue;
    }
    if (entry.isSymbolicLink()) {
      throw new Error(`Refuse renderer source symlink: ${name}`);
    }
    if (entry.isFile()) {
      files.push(name);
    }
  }
  return files.sort();
}

async function copyFile(source, destination) {
  const stat = await fs.lstat(source);
  if (stat.isSymbolicLink()) {
    throw new Error(`Refuse frontend source symlink: ${source}`);
  }
  if (!stat.isFile()) {
    throw new Error(`Frontend source is not a regular file: ${source}`);
  }
  await fs.mkdir(path.dirname(destination), {recursive: true});
  await fs.copyFile(source, destination);
}

export async function buildFrontend({
  desktopRoot = path.join(projectRoot, 'desktop'),
  vendorRoot = path.join(projectRoot, 'node_modules/@rhwp/core'),
  outputRoot = path.join(projectRoot, 'app/dist'),
  testBuild = false
} = {}) {
  const bridge = testBuild ? 'http-bridge.js' : 'tauri-bridge.js';
  const files = (await rendererFiles(desktopRoot)).filter(name =>
    !['http-bridge.js', 'tauri-bridge.js'].includes(name) || name === bridge);
  if (!files.includes(bridge)) {
    throw new Error(`Missing renderer bridge: ${bridge}`);
  }
  const html = await fs.readFile(path.join(desktopRoot, 'index.html'), 'utf8');
  const rendererScript = '<script type="module" src="renderer.js"></script>';
  if (!html.includes(rendererScript)) {
    throw new Error('Missing renderer module script');
  }
  const bundledHtml = html.replace(rendererScript, `<script src="${bridge}"></script>\n  ${rendererScript}`);
  const runtimeFiles = ['rhwp.js', 'rhwp_bg.wasm'];
  for (const name of runtimeFiles) {
    const stat = await fs.lstat(path.join(vendorRoot, name));
    if (stat.isSymbolicLink() || !stat.isFile()) {
      throw new Error(`Invalid rhwp runtime source (file or symlink): ${name}`);
    }
  }
  await fs.rm(outputRoot, {recursive: true, force: true});
  await fs.mkdir(outputRoot, {recursive: true});
  for (const name of files) {
    if (name !== 'index.html') {
      await copyFile(path.join(desktopRoot, name), path.join(outputRoot, name));
    }
  }
  await fs.writeFile(path.join(outputRoot, 'index.html'), bundledHtml);
  for (const name of runtimeFiles) {
    await copyFile(path.join(vendorRoot, name), path.join(outputRoot, 'vendor/rhwp', name));
  }
  return outputRoot;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    if (args.some(arg => arg !== '--test')) throw new Error('Usage: build-frontend.mjs [--test]');
    const output = await buildFrontend({testBuild: args.includes('--test')});
    console.log(`Frontend built: ${output}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
