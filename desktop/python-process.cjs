const {spawn} = require('node:child_process');
const runtime = require('./runtime.cjs');

// Main passes Electron at startup. Unit tests run without Electron, as a
// source checkout would.
let host = {app: {isPackaged: false}};
let python;

function useElectron(electron) {
  host = electron;
  python = undefined;
}

// The installed app carries its own Python; a source checkout needs 3.11+.
function pythonExecutable() {
  if (python === undefined) {
    python = runtime.pythonCommand(host);
  }
  if (!python) {
    throw Error('HWP Workbench needs Python 3.11 or newer / Python 3.11 이상이 필요합니다');
  }
  return python;
}

function engineSpawnOptions() {
  return {cwd: runtime.appRoot(host), shell: false, env: runtime.engineEnvironment(host), stdio: ['pipe', 'pipe', 'pipe']};
}

// Main supplies fixed actions and argument lists. This is never exposed over IPC.
function runPython(arguments_, input = '', maxBytes = 32 * 1024 * 1024) {
  return new Promise((resolve, reject) => {
    const child = spawn(pythonExecutable(), arguments_, engineSpawnOptions());
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

module.exports = {runPython, useElectron, pythonExecutable, engineSpawnOptions};
