const fs = require('node:fs/promises');
const {constants} = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const MAX_DOCUMENT_BYTES = 64 * 1024 * 1024;

async function readDocumentBytes(file, expectedHash) {
  // Reject symlink components as well as a symlink at the granted filename.
  const absolute = path.resolve(file);
  let component = path.parse(absolute).root;
  for (const name of absolute.slice(component.length).split(path.sep)) {
    component = path.join(component, name);
    if ((await fs.lstat(component)).isSymbolicLink()) {
      throw Error('Source path changed');
    }
  }
  const handle = await fs.open(absolute, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const stat = await handle.stat();
    if (!stat.isFile() || stat.size < 1 || stat.size > MAX_DOCUMENT_BYTES) {
      throw Error('File exceeds 64 MiB');
    }
    // Bound the read even if another process grows the file after stat().
    const data = Buffer.alloc(stat.size);
    let offset = 0;
    while (offset < data.length) {
      const {bytesRead} = await handle.read(data, offset, data.length - offset, offset);
      if (!bytesRead) {
        throw Error('Source changed');
      }
      offset += bytesRead;
    }
    const extra = Buffer.alloc(1);
    const {bytesRead} = await handle.read(extra, 0, 1, offset);
    const hash = crypto.createHash('sha256').update(data).digest('hex');
    if (bytesRead || hash !== expectedHash) {
      throw Error('Source changed');
    }
    return new Uint8Array(data);
  } finally {
    await handle.close();
  }
}

module.exports = {readDocumentBytes};
