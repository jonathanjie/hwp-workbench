const fs = require('node:fs/promises');
const path = require('node:path');
const {runPython} = require('./python-process.cjs');
const ROOT = path.resolve(__dirname, '..');
const BUNDLE = path.join(ROOT, 'dist', 'hwp-workbench.mcpb');

async function bundleAvailable() {
  try {
    const folder = await fs.lstat(path.dirname(BUNDLE));
    const file = await fs.lstat(BUNDLE);
    if (!folder.isDirectory() || folder.isSymbolicLink() || !file.isFile() || file.isSymbolicLink()) {
      throw Error('Unsafe bundle path');
    }
    return true;
  } catch (error) {
    if (error.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

class AgentTools {
  constructor({documentRoot, copy, reveal}) {
    this.documentRoot = documentRoot;
    this.copy = copy;
    this.reveal = reveal;
  }

  async status(...args) {
    if (args.length) {
      throw Error('Status accepts no arguments');
    }
    const result = JSON.parse(await runPython(['-m', 'hwp_workbench', 'skill', 'status']));
    return {...result, claudeDesktop: {installed: null, bundleAvailable: await bundleAvailable()}, hasDocument: !!this.documentRoot()};
  }

  async install(target) {
    if (!['claude', 'codex'].includes(target)) {
      throw Error('Invalid skill target');
    }
    await runPython(['-m', 'hwp_workbench', 'skill', 'install', '--target', target]);
    return this.status();
  }

  async copyMcp(client) {
    if (!['claude-code', 'codex'].includes(client)) {
      throw Error('Invalid MCP client');
    }
    const root = this.documentRoot();
    if (!root) {
      throw Error('No document');
    }
    const command = (await runPython(['-m', 'hwp_workbench', 'mcp-config', '--client', client, '--root', root])).trim();
    this.copy(command);
    return command;
  }

  async bundle(...args) {
    if (args.length) {
      throw Error('Bundle reveal accepts no arguments');
    }
    if (!(await bundleAvailable())) {
      try {
        const folder = await fs.lstat(path.dirname(BUNDLE));
        if (!folder.isDirectory() || folder.isSymbolicLink()) {
          throw Error('Unsafe bundle directory');
        }
      } catch (error) {
        if (error.code !== 'ENOENT') {
          throw error;
        }
      }
      await runPython([path.join(ROOT, 'scripts', 'build-mcpb.py')]);
    }
    if (!(await bundleAvailable())) {
      throw Error('Bundle unavailable');
    }
    this.reveal(BUNDLE);
    return true;
  }
}

module.exports = {AgentTools};
