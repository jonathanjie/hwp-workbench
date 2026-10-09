const fs = require('node:fs/promises');
const {constants} = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const MAX_RECENT = 8;
const MAX_PNG_BYTES = 128 * 1024;
const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const VALID_ID = /^[a-f0-9]{32}$/;

async function validateDroppedFile(file) {
  if (typeof file !== 'string' || !path.isAbsolute(file) || !/\.hwpx?$/i.test(file)) {
    throw Error('Unsupported document');
  }
  // lstat does not follow a final symlink, so a linked document is refused.
  // A symlinked parent folder (for example /tmp on macOS) is allowed; the
  // caller gets the canonical path.
  const stat = await fs.lstat(file);
  if (!stat.isFile()) {
    throw Error('Document must be a regular file without symlinks');
  }
  return fs.realpath(file);
}

async function readBounded(file, limit) {
  const handle = await fs.open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const stat = await handle.stat();
    if (!stat.isFile() || stat.size > limit) {
      throw Error('Recent data limit exceeded');
    }
    const buffer = Buffer.alloc(limit + 1);
    let size = 0;
    while (size < buffer.length) {
      const {bytesRead} = await handle.read(buffer, size, buffer.length - size, null);
      if (!bytesRead) {
        break;
      }
      size += bytesRead;
    }
    if (size > limit) {
      throw Error('Recent data limit exceeded');
    }
    return buffer.subarray(0, size);
  } finally {
    await handle.close();
  }
}

class RecentStore {
  constructor(directory) {
    this.directory = path.resolve(directory);
    this.index = path.join(this.directory, 'recent.json');
    this.thumbs = path.join(this.directory, 'thumbs');
  }

  async ensureDirectory(directory) {
    await fs.mkdir(directory, {recursive: true, mode: 0o700});
    // Refuse a store folder that is itself a symlink. Parent folders may be
    // symlinks: the macOS temporary folder and some home folders are.
    if (!(await fs.lstat(directory)).isDirectory()) {
      throw Error('Recent directory changed');
    }
  }

  async entries() {
    await this.ensureDirectory(this.directory);
    let entries;
    try {
      entries = JSON.parse((await readBounded(this.index, 64 * 1024)).toString('utf8'));
    } catch (error) {
      if (error.code === 'ENOENT') {
        return [];
      }
      throw error;
    }
    if (!Array.isArray(entries) || entries.length > MAX_RECENT || entries.some(entry =>
      !entry || typeof entry.id !== 'string' || !VALID_ID.test(entry.id) || typeof entry.file !== 'string' ||
      !path.isAbsolute(entry.file) || !/\.hwpx?$/i.test(entry.file) || typeof entry.openedAt !== 'string' ||
      !['hwp', 'hwpx'].includes(entry.format))) {
      throw Error('Invalid recent data');
    }
    return entries;
  }

  async atomicWrite(file, data) {
    const temporary = file + '.' + crypto.randomBytes(8).toString('hex') + '.tmp';
    try {
      await fs.writeFile(temporary, data, {flag: 'wx', mode: 0o600});
      await fs.rename(temporary, file);
    } finally {
      await fs.rm(temporary, {force: true});
    }
  }

  async record(file, format) {
    const entries = await this.entries();
    const previous = entries.find(entry => entry.file === file);
    const id = previous?.id || crypto.randomBytes(16).toString('hex');
    const entry = {id, file, name: path.basename(file), format, openedAt: new Date().toISOString()};
    const next = [entry, ...entries.filter(item => item.file !== file)].slice(0, MAX_RECENT);
    await this.atomicWrite(this.index, JSON.stringify(next, null, 2));
    // Keep the thumbnail cache bounded to the same eight grants.
    for (const removed of entries.filter(item => !next.some(current => current.id === item.id))) {
      await this.ensureDirectory(this.thumbs);
      await fs.rm(path.join(this.thumbs, removed.id + '.png'), {force: true});
    }
    return id;
  }

  async entryFor(id) {
    if (typeof id !== 'string' || !VALID_ID.test(id)) {
      throw Error('Invalid recent document');
    }
    const entry = (await this.entries()).find(entry => entry.id === id);
    if (!entry) {
      throw Error('Invalid recent document');
    }
    return entry;
  }

  async fileFor(id) {
    const entry = await this.entryFor(id);
    return validateDroppedFile(entry.file);
  }

  async list() {
    const entries = await this.entries();
    await this.ensureDirectory(this.thumbs);
    return Promise.all(entries.map(async entry => {
      let thumbnail = null;
      try {
        const png = await readBounded(path.join(this.thumbs, entry.id + '.png'), MAX_PNG_BYTES);
        thumbnail = `data:image/png;base64,${png.toString('base64')}`;
      } catch {}
      return {id: entry.id, name: path.basename(entry.file), format: entry.format, openedAt: entry.openedAt, thumbnail};
    }));
  }

  async cacheThumbnail(id, data) {
    await this.entryFor(id);
    if (typeof data !== 'string' || !data.startsWith('data:image/png;base64,') || data.length > MAX_PNG_BYTES * 4 / 3 + 30) {
      throw Error('Invalid thumbnail');
    }
    const png = Buffer.from(data.slice('data:image/png;base64,'.length), 'base64');
    if (png.length < 24 || png.length > MAX_PNG_BYTES || !png.subarray(0, 8).equals(PNG_SIGNATURE) ||
        png.toString('ascii', 12, 16) !== 'IHDR' || !png.readUInt32BE(16) || !png.readUInt32BE(20) ||
        png.readUInt32BE(16) > 256 || png.readUInt32BE(20) > 384) {
      throw Error('Invalid thumbnail');
    }
    await this.ensureDirectory(this.thumbs);
    await this.atomicWrite(path.join(this.thumbs, id + '.png'), png);
    return true;
  }
}

module.exports = {RecentStore, validateDroppedFile};
