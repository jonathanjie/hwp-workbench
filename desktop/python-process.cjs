const {spawn} = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const ROOT = path.resolve(__dirname, '..');

function pythonExecutable() {
  return process.env.HWP_WORKBENCH_PYTHON ||
    ['/opt/homebrew/bin/python3', '/usr/local/bin/python3', '/usr/bin/python3'].find(file => fs.existsSync(file)) || 'python3';
}

// Main supplies fixed actions and argument lists. This is never exposed over IPC.
function runPython(arguments_, input = '', maxBytes = 32 * 1024 * 1024) {
  return new Promise((resolve, reject) => {
    const child = spawn(pythonExecutable(), arguments_, {
      cwd: ROOT, shell: false,
      env: {...process.env, PYTHONPATH: path.join(ROOT, 'engine'), PYTHONUTF8: '1'},
      stdio: ['pipe', 'pipe', 'pipe']
    });
    let output = '';
    let errors = '';
    let size = 0;
    let settled = false;
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      finish(Error('Processing timed out'));
    }, 30000);
    function finish(error, value) {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      error ? reject(error) : resolve(value);
    }
    child.on('error', () => finish(Error('Python unavailable')));
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', data => {
      size += Buffer.byteLength(data);
      if (size > maxBytes) {
        child.kill('SIGKILL');
        finish(Error('Response too large'));
      } else {
        output += data;
      }
    });
    child.stderr.on('data', data => {
      if (errors.length < 65536) {
        errors += data;
      }
    });
    child.stdin.on('error', () => {});
    child.on('close', code => {
      if (code !== 0) {
        let message = 'Engine failed';
        try {
          message = JSON.parse(errors).error.message;
        } catch {}
        finish(Error(message));
      } else {
        finish(null, output);
      }
    });
    child.stdin.end(input);
  });
}

module.exports = {runPython};
