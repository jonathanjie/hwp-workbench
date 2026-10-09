const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const {AgentTools} = require('../agent-tools.cjs');

test('fixed skill actions install only into a temporary home and MCP roots come from main', async () => {
  const home = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'hwp-tools-')));
  const previous = process.env.HWP_WORKBENCH_SKILL_HOME;
  process.env.HWP_WORKBENCH_SKILL_HOME = home;
  let root = null;
  const tools = new AgentTools({documentRoot: () => root, copy: value => value, reveal: () => {}});
  try {
    const status = await tools.status();
    assert.equal(status.claude.installed, false);
    assert.equal(status.codex.installed, false);
    await assert.rejects(tools.install('other'), /Invalid/);
    await tools.install('codex');
    assert.equal((await tools.status()).codex.installed, true);
    assert.equal((await tools.status()).claude.installed, false);
    await assert.rejects(tools.copyMcp('codex'), /No document/);
    root = home;
    const line = await tools.copyMcp('claude-code');
    assert.match(line, /claude mcp add hwp-workbench/);
    assert.ok(line.includes(home));
    await assert.rejects(tools.copyMcp('shell'), /Invalid/);
    await assert.rejects(tools.status('extra'), /arguments/);
    await assert.rejects(tools.bundle('extra'), /arguments/);
  } finally {
    if (previous === undefined) {
      delete process.env.HWP_WORKBENCH_SKILL_HOME;
    } else {
      process.env.HWP_WORKBENCH_SKILL_HOME = previous;
    }
    await fs.rm(home, {recursive: true, force: true});
  }
});
