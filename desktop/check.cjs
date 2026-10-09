const fs = require('node:fs');
const path = require('node:path');
const {spawnSync} = require('node:child_process');

const files = fs.readdirSync(__dirname, {recursive: true});
for (const file of files) {
  if (/\.(?:js|cjs|mjs)$/.test(file)) {
    const result = spawnSync(process.execPath, ['--check', path.join(__dirname, file)], {stdio: 'inherit'});
    if (result.status !== 0) {
      process.exit(result.status ?? 1);
    }
  }
}
