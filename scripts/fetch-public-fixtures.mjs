// Download fixed Git blobs. Fixtures stay local and are never redistributed.
import fs from 'node:fs/promises';
import {constants} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash, randomBytes} from 'node:crypto';

const root = fileURLToPath(new URL('../', import.meta.url));
const maxResponseBytes = 4 * 1024 * 1024;

async function boundedResponse(response) {
  if (!response.ok) throw Error(`Download returned HTTP ${response.status}`);
  const chunks = [];
  let total = 0;
  for await (const chunk of response.body) {
    total += chunk.length;
    if (total > maxResponseBytes) throw Error('Blob response exceeds 4 MiB');
    chunks.push(chunk);
  }
  const record = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  if (record.encoding !== 'base64' || typeof record.content !== 'string') throw Error('Unexpected blob encoding');
  return Buffer.from(record.content, 'base64');
}

async function readExisting(destination, expectedSize) {
  let handle;
  try {
    handle = await fs.open(destination, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
  try {
    const stat = await handle.stat();
    if (!stat.isFile() || stat.size !== expectedSize) throw Error('Existing fixture is not a regular file of the expected size');
    const data = Buffer.alloc(expectedSize + 1);
    let offset = 0;
    while (offset < data.length) {
      const {bytesRead} = await handle.read(data, offset, data.length - offset, null);
      if (bytesRead === 0) break;
      offset += bytesRead;
    }
    if (offset !== expectedSize) throw Error('Existing fixture changed while reading');
    return data.subarray(0, offset);
  } finally {
    await handle.close();
  }
}

async function publishNew(destination, data) {
  const temporary = `${destination}.${randomBytes(8).toString('hex')}.tmp`;
  // Own the temporary file before entering cleanup. An exclusive link publishes
  // complete bytes and refuses any destination created during the download.
  const handle = await fs.open(temporary, 'wx', 0o600);
  try {
    await handle.writeFile(data);
    await handle.sync();
    await fs.link(temporary, destination);
  } finally {
    await handle.close();
    await fs.unlink(temporary);
  }
}

export async function fetchFixtures({manifest, folder, fetchBlob = fetch}) {
  await fs.mkdir(folder, {recursive: true});
  if ((await fs.lstat(folder)).isSymbolicLink()) throw Error('Refuse fixture folder symlink');
  const results = [];
  for (const item of manifest.fixtures) {
    try {
      if (!/^[\w.-]+$/.test(item.name) || ['.', '..'].includes(item.name)) throw Error('Unsafe fixture name');
      if (!Number.isSafeInteger(item.bytes) || item.bytes < 1 || item.bytes > maxResponseBytes) throw Error('Invalid fixture size');
      if (!/^[\w.-]+\/[\w.-]+$/.test(item.repository) || !/^[a-f0-9]{40}$/.test(item.blobSha1)) throw Error('Invalid blob identity');
      const destination = path.join(folder, item.name);
      const existing = await readExisting(destination, item.bytes);
      const data = existing ?? await boundedResponse(await fetchBlob(
        `https://api.github.com/repos/${item.repository}/git/blobs/${item.blobSha1}`,
        {headers: {Accept: 'application/vnd.github+json', 'User-Agent': 'hwp-workbench-fixture-fetch'}, signal: AbortSignal.timeout(20000)}
      ));
      const blob = createHash('sha1').update(`blob ${data.length}\0`).update(data).digest('hex');
      if (blob !== item.blobSha1 || data.length !== item.bytes) throw Error('Blob identity or byte count mismatch');
      const sha256 = createHash('sha256').update(data).digest('hex');
      if (item.sha256 && sha256 !== item.sha256) throw Error('SHA-256 mismatch');
      if (!existing) await publishNew(destination, data);
      results.push({name: item.name, status: 'verified', sha256, bytes: data.length});
    } catch (error) {
      results.push({name: item.name, status: 'download-failed', error: error.message});
    }
  }
  return results;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw Error('Usage: node scripts/fetch-public-fixtures.mjs');
    const manifest = JSON.parse(await fs.readFile(path.join(root, 'fixtures/public-manifest.json'), 'utf8'));
    const results = await fetchFixtures({manifest, folder: path.join(root, 'fixtures/public')});
    const outputs = path.join(root, 'outputs');
    await fs.mkdir(outputs, {recursive: true});
    const report = path.join(outputs, `public-fixture-downloads-${Date.now()}-${randomBytes(3).toString('hex')}.json`);
    await fs.writeFile(report, JSON.stringify(results, null, 2) + '\n', {flag: 'wx', mode: 0o600});
    console.log(JSON.stringify({report, results}, null, 2));
    process.exitCode = results.every(item => item.status === 'verified') ? 0 : 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
