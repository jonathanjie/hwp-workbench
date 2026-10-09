import {element} from './dom.js';
import {t} from './i18n.js';

const CLIENTS = [
  {name: 'Claude Code', key: 'claude', target: 'claude', client: 'claude-code', glyph: 'C'},
  {name: 'Codex', key: 'codex', target: 'codex', client: 'codex', glyph: 'X'},
  {name: 'Claude Desktop', key: 'claudeDesktop', glyph: 'C'}
];

export class AgentsCard {
  constructor(hosts, call, toast, onError) {
    this.hosts = hosts;
    this.call = call;
    this.toast = toast;
    this.onError = onError;
    this.state = null;
  }

  async refresh() {
    try {
      this.state = await this.call('agentsStatus');
    } catch {
      this.state = null;
    }
    this.render();
  }

  async action(method, argument) {
    if (this.busy) {
      return;
    }
    this.busy = true;
    this.render();
    try {
      const result = argument === undefined ? await this.call(method) : await this.call(method, argument);
      if (method === 'installSkill') {
        this.state = result;
        this.toast.show('skillInstalled');
      } else if (method === 'copyMcp') {
        this.toast.show('copied');
      }
    } catch (error) {
      this.onError(error);
    } finally {
      this.busy = false;
      this.render();
    }
  }

  render() {
    for (const host of this.hosts) {
      const card = element('aside', 'agents');
      const label = element('div', 'lbl');
      label.append(element('span', '', t('agents')), element('span', '', t('local')));
      card.append(label);
      for (const client of CLIENTS) {
        const status = this.state?.[client.key];
        const row = element('div', 'agent-row');
        const avatar = element('span', `av ${client.key}`, client.glyph);
        const name = element('span', 'agent-name', client.name);
        const statusKey = client.target ? !status ? 'unavailable' : status.installed ? 'skill' : 'off' : 'unverified';
        row.append(avatar, name, element('span', status?.installed ? 'st' : 'st off', t(statusKey)));
        const actions = element('div', 'agent-actions');
        if (client.target) {
          const install = element('button', 'btn ghost', t('install'));
          install.disabled = this.busy;
          install.onclick = () => this.action('installSkill', client.target);
          const copy = element('button', 'btn line', t('copyMcp'));
          copy.disabled = this.busy || !this.state?.hasDocument;
          copy.onclick = () => this.action('copyMcp', client.client);
          actions.append(install, copy);
        } else {
          const bundle = element('button', 'btn line', t('showBundle'));
          bundle.disabled = this.busy;
          bundle.onclick = () => this.action('revealBundle');
          actions.append(bundle);
        }
        card.append(row, actions);
      }
      host.replaceChildren(card);
    }
  }
}
