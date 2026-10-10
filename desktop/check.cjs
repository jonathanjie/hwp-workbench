const fs = require('node:fs');
const path = require('node:path');
const {spawnSync} = require('node:child_process');

const root = path.resolve(__dirname, '..');
const files = ['desktop', 'scripts', 'tests/packaging', 'tests/qa'].flatMap(folder =>
  fs.readdirSync(path.join(root, folder), {recursive: true}).map(file => path.join(folder, file)));
for (const file of files) {
  if (/\.(?:js|cjs|mjs)$/.test(file)) {
    const result = spawnSync(process.execPath, ['--check', path.join(root, file)], {stdio: ['ignore', 'inherit', 'inherit']});
    if (result.status !== 0) {
      process.exit(result.status ?? 1);
    }
  }
}
