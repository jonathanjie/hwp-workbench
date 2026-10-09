const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const {readDocumentBytes} = require('../document-bytes.cjs');

test('document bytes reject stale sources, oversized files and replaced symlink grants', async () => {
  // realpath avoids macOS /var and /tmp aliases in the test-owned directory.
  const directory = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-bytes-')));
  try {
    const file = path.join(directory, 'source.hwpx');
    const data = Buffer.from('document');
    const hash = crypto.createHash('sha256').update(data).digest('hex');
    await fs.writeFile(file, data);
    assert.deepEqual(Buffer.from(await readDocumentBytes(file, hash)), data);
    await fs.writeFile(file, 'changed');
    await assert.rejects(readDocumentBytes(file, hash), /Source changed/);
    await fs.rm(file);
    await fs.writeFile(path.join(directory, 'other.hwpx'), data);
    await fs.symlink(path.join(directory, 'other.hwpx'), file);
    await assert.rejects(readDocumentBytes(file, hash), /Source path changed/);
    await fs.rm(file);
    const handle = await fs.open(file, 'w');
    await handle.truncate(64 * 1024 * 1024 + 1);
    await handle.close();
    await assert.rejects(readDocumentBytes(file, hash), /64 MiB/);
  } finally {
    await fs.rm(directory, {recursive: true, force: true});
  }
});
