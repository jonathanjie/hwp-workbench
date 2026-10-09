const path = require('node:path');
const {runPython} = require('./python-process.cjs');

class AgentFiles {
  constructor(root, name) {
    this.root = root;
    this.name = name;
  }

  async invoke(action, record) {
    const payload = JSON.stringify(record);
    if (Buffer.byteLength(payload) > 2 * 1024 * 1024) {
      throw Error('Link request too large');
    }
    return JSON.parse(await runPython([path.join(__dirname, 'agent-files.py'), this.root, action, this.name], payload));
  }

  read(sha256) {
    return this.invoke('read', {sha256});
  }

  session(record) {
    return this.invoke('session', record);
  }

  outcome(file, acceptedNodeIds, declinedNodeIds, expectedPatch) {
    return this.invoke('outcome', {file, acceptedNodeIds, declinedNodeIds, expectedPatch});
  }
}

module.exports = {AgentFiles};
