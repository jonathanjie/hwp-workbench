const fs = require('node:fs/promises');
const fsSync = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {AgentFiles} = require('./agent-files.cjs');

class AgentInbox {
  constructor({plan, send, preferences}) {
    this.plan = plan;
    this.send = send;
    this.preferences = preferences;
    this.document = null;
    this.enabled = false;
    this.rows = [];
    this.decisions = new Map();
    this.validated = new Map();
    this.generation = 0;
    this.scanning = null;
  }

  stop() {
    this.generation += 1;
    this.watcher?.close();
    clearInterval(this.poll);
    clearTimeout(this.sessionTimer);
    this.watcher = null;
    this.poll = null;
  }

  async setDocument(document) {
    this.stop();
    this.document = document;
    this.files = new AgentFiles(document.root, document.name);
    this.rows = [];
    this.decisions.clear();
    this.validated.clear();
    this.selection = null;
    this.staged = null;
    let settings = {};
    try {
      const info = await fs.lstat(this.preferences);
      if (info.isFile() && !info.isSymbolicLink() && info.size <= 1024 * 1024) {
        const handle = await fs.open(this.preferences, fsSync.constants.O_RDONLY | fsSync.constants.O_NOFOLLOW);
        try {
          settings = JSON.parse(await handle.readFile('utf8'));
        } finally {
          await handle.close();
        }
      }
    } catch {}
    this.settings = settings && typeof settings === 'object' && !Array.isArray(settings) ? settings : {};
    this.enabled = this.settings[document.file] === true;
    if (this.enabled) {
      await this.start();
    }
    this.emit();
  }

  state() {
    return {enabled: this.enabled, proposals: this.enabled ? this.rows : [], error: this.error || null};
  }

  emit() {
    this.send(this.state());
  }

  async toggle(enabled) {
    if (typeof enabled !== 'boolean' || !this.document) {
      throw Error('Invalid Agent link switch');
    }
    this.stop();
    this.enabled = enabled;
    this.settings[this.document.file] = enabled;
    const temporary = this.preferences + '.' + crypto.randomBytes(8).toString('hex');
    try {
      await fs.writeFile(temporary, JSON.stringify(this.settings), {flag: 'wx', mode: 0o600});
      try {
        const info = await fs.lstat(this.preferences);
        if (!info.isFile() || info.isSymbolicLink()) {
          throw Error('Unsafe Agent link settings');
        }
      } catch (error) {
        if (error.code !== 'ENOENT') {
          throw error;
        }
      }
      await fs.rename(temporary, this.preferences);
    } finally {
      await fs.rm(temporary, {force: true});
    }
    if (enabled) {
      await this.start();
    }
    this.emit();
    return this.state();
  }

  record() {
    const document = this.document;
    return {schemaVersion: 1, app: 'hwp-workbench', appVersion: '0.1.0-alpha.1',
      document: {name: document.name, format: document.format, sha256: document.sha256},
      selection: this.selection, staged: this.staged, updatedAt: new Date().toISOString()};
  }

  update(selection, staged) {
    this.selection = selection;
    this.staged = staged?.operations.length ? staged : null;
    if (!this.enabled) {
      return;
    }
    clearTimeout(this.sessionTimer);
    const files = this.files;
    const generation = this.generation;
    this.sessionTimer = setTimeout(async () => {
      if (generation !== this.generation || !this.enabled) {
        return;
      }
      try {
        await files.session(this.record());
      } catch (error) {
        this.error = error.message;
        this.emit();
      }
    }, 300);
  }

  async start() {
    try {
      await this.files.session(this.record());
      await this.scan();
      const folder = path.join(this.document.root, '.hwp-workbench', 'proposals');
      // Watch events are hints only. All reads still use no-follow descriptors.
      const info = await fs.lstat(folder);
      if (info.isDirectory() && !info.isSymbolicLink()) {
        try {
          this.watcher = fsSync.watch(folder, () => this.scan());
          this.watcher.on('error', () => {
            this.watcher?.close();
            this.watcher = null;
          });
        } catch {}
      }
    } catch (error) {
      this.error = error.message;
    }
    this.poll = setInterval(() => this.scan(), 2000);
    this.poll.unref();
  }

  // A request during a running scan waits for it and one more pass, so
  // `await scan()` always reflects files written before the call.
  scan() {
    if (!this.enabled) {
      return Promise.resolve();
    }
    if (this.scanning) {
      this.rescan = true;
      return this.scanning;
    }
    this.scanning = this.scanUntilCurrent().finally(() => {
      this.scanning = null;
    });
    return this.scanning;
  }

  async scanUntilCurrent() {
    do {
      this.rescan = false;
      await this.scanOnce();
    } while (this.rescan && this.enabled);
  }

  async scanOnce() {
    const generation = this.generation;
    const document = this.document;
    try {
      const rows = await this.files.read(document.sha256);
      for (const row of rows) {
        if (row.status !== 'pending') {
          continue;
        }
        const fingerprint = crypto.createHash('sha256').update(JSON.stringify(row.patch)).digest('hex');
        if (this.validated.get(row.file) !== fingerprint) {
          this.decisions.delete(row.file);
          try {
            await this.plan(document, row.patch);
            this.validated.set(row.file, fingerprint);
          } catch (error) {
            row.status = /hash|source changed/i.test(error.message) ? 'stale' : 'unreadable';
            delete row.patch;
          }
        }
        row.decisions = this.decisions.get(row.file) || {};
      }
      if (generation === this.generation) {
        this.rows = rows;
        this.error = null;
        this.emit();
      }
    } catch (error) {
      if (generation === this.generation) {
        this.error = error.message;
        this.emit();
      }
    }
  }

  async decide(file, ids, accepted) {
    if (!this.enabled || !Array.isArray(ids) || !ids.length || ids.length > 1000 || typeof accepted !== 'boolean') {
      throw Error('Invalid proposal decision');
    }
    // Re-read before every decision: proposals and sources can change externally.
    const rows = await this.files.read(this.document.sha256);
    const row = rows.find(row => row.file === file);
    const displayed = this.rows.find(row => row.file === file);
    if (row?.status !== 'pending' || !row.patch || JSON.stringify(row.patch) !== JSON.stringify(displayed?.patch)) {
      throw Error('Stale proposal · reopen');
    }
    await this.plan(this.document, row.patch);
    const decisions = {...(this.decisions.get(file) || {})};
    const operations = row.patch.operations;
    const targets = new Set(ids);
    if (targets.size !== ids.length || ids.some(id => decisions[id] || !operations.some(operation => operation.nodeId === id))) {
      throw Error('Invalid or repeated proposal decision');
    }
    ids.forEach(id => { decisions[id] = accepted ? 'accepted' : 'declined'; });
    const acceptedNodeIds = Object.keys(decisions).filter(id => decisions[id] === 'accepted');
    const declinedNodeIds = Object.keys(decisions).filter(id => decisions[id] === 'declined');
    if (Object.keys(decisions).length === operations.length) {
      await this.files.outcome(file, acceptedNodeIds, declinedNodeIds, row.patch);
    }
    this.decisions.set(file, decisions);
    await this.scan();
    return {author: row.author, operations: accepted ? operations.filter(operation => targets.has(operation.nodeId)) : []};
  }
}

module.exports = {AgentInbox};
