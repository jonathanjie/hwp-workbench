/** Runtime paths shared by the desktop's engine and Agents card.
 * Electron is supplied by the caller so this module can be tested without a GUI.
 */
const path = require('node:path');
const os = require('node:os');
const { spawnSync } = require('node:child_process');

function appRoot(electron, options = {}) {
  if (electron.app.isPackaged) {
    return path.join(options.resourcesPath ?? process.resourcesPath, 'app');
  }
  return path.resolve(__dirname, '..');
}

function pythonCommand(electron, options = {}) {
  const environment = options.environment ?? process.env;
  if (environment.HWP_WORKBENCH_PYTHON) {
    return environment.HWP_WORKBENCH_PYTHON;
  }
  if (electron.app.isPackaged) {
    return path.join(options.resourcesPath ?? process.resourcesPath, 'python', 'bin', 'python3');
  }
  const home = options.home ?? os.homedir();
  const candidates = [
    '/opt/homebrew/bin/python3',
    '/usr/local/bin/python3',
    path.join(home, '.local', 'bin', 'python3'),
    path.join(home, '.pyenv', 'shims', 'python3'),
    '/Library/Frameworks/Python.framework/Versions/Current/bin/python3',
    '/opt/miniconda3/bin/python3',
    '/usr/bin/python3',
    'python3',
  ];
  const probe = options.spawnSync ?? spawnSync;
  for (const candidate of candidates) {
    const result = probe(candidate, ['-c', 'import sys; print(sys.version_info[0], sys.version_info[1])'], {
      env: environment,
      encoding: 'utf8',
      shell: false,
      timeout: 10000,
      maxBuffer: 4096,
      windowsHide: true,
    });
    if (result.error || result.status !== 0) {
      continue;
    }
    const version = /^(\d+)\s+(\d+)\s*$/.exec(result.stdout ?? '');
    if (version && (Number(version[1]) > 3 || (Number(version[1]) === 3 && Number(version[2]) >= 11))) {
      return candidate;
    }
  }
  return null;
}

function engineEnvironment(electron, options = {}) {
  const environment = {
    ...(options.environment ?? process.env),
    PYTHONPATH: path.join(appRoot(electron, options), 'engine'),
    PYTHONUTF8: '1',
    PYTHONDONTWRITEBYTECODE: '1',
  };
  // Electron supplies Node in an installed app; no external Node is required.
  if (!environment.HWP_WORKBENCH_NODE) {
    environment.HWP_WORKBENCH_NODE = options.execPath ?? process.execPath;
    environment.ELECTRON_RUN_AS_NODE = '1';
  }
  return environment;
}

function cliPath(electron, options = {}) {
  return path.join(appRoot(electron, options), 'scripts', 'hwp-workbench');
}

function bundlePath(electron, options = {}) {
  return path.join(appRoot(electron, options), 'dist', 'hwp-workbench.mcpb');
}

module.exports = { appRoot, pythonCommand, engineEnvironment, cliPath, bundlePath };
