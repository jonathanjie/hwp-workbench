import {element} from './dom.js';
import {t, currentLanguage} from './i18n.js';
import {wordDiffSegments} from './changes.js';

function relativeTime(date) {
  const seconds = Math.max(0, Math.floor((Date.now() - Date.parse(date)) / 1000));
  if (!Number.isFinite(seconds)) {
    return t('timeUnavailable');
  }
  const unit = seconds < 60 ? 'second' : seconds < 3600 ? 'minute' : seconds < 86400 ? 'hour' : 'day';
  const amount = Math.floor(seconds / ({second: 1, minute: 60, hour: 3600, day: 86400}[unit]));
  return new Intl.RelativeTimeFormat(currentLanguage(), {numeric: 'auto'}).format(-amount, unit);
}

function action(label, accept, ids, file, decide, disabled) {
  const button = element('button', accept ? 'btn ink' : 'btn line', t(label));
  button.disabled = disabled;
  button.onclick = () => decide(file, ids, accept);
  return button;
}

export class AgentPanel {
  constructor({host, switchControl, chip, decide, toggle, toast}) {
    this.host = host;
    this.switchControl = switchControl;
    this.chip = chip;
    this.decide = decide;
    this.toast = toast;
    this.seen = new Set();
    this.state = {enabled: false, proposals: []};
    switchControl.onchange = () => toggle(switchControl.checked);
  }

  reset(state) {
    this.seen.clear();
    this.update(state || {enabled: false, proposals: []}, false);
  }

  update(state, announce = true) {
    this.state = state;
    for (const row of state.proposals) {
      if (!this.seen.has(row.file) && row.status === 'pending' && announce) {
        this.toast.show('agentProposed', {author: row.author, n: row.operations});
      }
      this.seen.add(row.file);
    }
    this.render();
  }

  render() {
    const state = this.state;
    this.switchControl.checked = state.enabled;
    const pending = state.proposals.filter(row => row.status === 'pending');
    this.chip.hidden = !state.enabled || !pending.length;
    this.chip.replaceChildren(element('span', 'dot'), document.createTextNode(t('agentCount', {n: pending.length})));
    const cards = state.enabled ? state.proposals.map(row => this.card(row)) : [];
    if (state.error) {
      cards.unshift(element('div', 'agent-error', t('agentLinkFailed')));
    }
    this.host.replaceChildren(...cards);
  }

  card(row) {
    const card = element('article', 'change agent proposal');
    card.dataset.proposal = row.file;
    const header = element('div', 'head');
    header.append(element('span', '', row.author || t('agent')), element('span', 'who', row.createdAt ? relativeTime(row.createdAt) : t('timeUnavailable')));
    card.append(header, element('div', 'proposal-title', row.title || t('unreadable')));
    const steps = element('div', 'steps');
    steps.append(element('span', 'done', t('agentRead')), element('span', 'done', t('agentProposal')),
      element('span', 'now', t(row.status === 'pending' ? 'agentWaiting' : row.status === 'partial' ? 'proposalPartial' : row.status)));
    card.append(steps);
    const operations = row.patch?.operations || [];
    for (const operation of operations) {
      const change = element('div', 'proposal-change');
      change.dataset.nodeId = operation.nodeId;
      const diff = element('div', 'diff');
      for (const segment of wordDiffSegments(operation.expectedText, operation.text)) {
        if (segment.type === 'same') {
          diff.append(document.createTextNode(segment.text));
        } else {
          diff.append(element(segment.type, '', segment.text));
        }
      }
      change.append(diff);
      if (row.status === 'pending') {
        const decision = row.decisions?.[operation.nodeId];
        if (decision) {
          change.append(element('span', 'tag off', t(decision)));
        } else {
          const actions = element('div', 'acts');
          actions.append(action('apply', true, [operation.nodeId], row.file, this.decide, this.busy),
            action('decline', false, [operation.nodeId], row.file, this.decide, this.busy));
          change.append(actions);
        }
      }
      card.append(change);
    }
    const remaining = operations.filter(operation => !row.decisions?.[operation.nodeId]).map(operation => operation.nodeId);
    if (row.status === 'pending' && remaining.length) {
      const actions = element('div', 'acts proposal-actions');
      actions.append(action('applyAll', true, remaining, row.file, this.decide, this.busy),
        action('declineAll', false, remaining, row.file, this.decide, this.busy));
      card.append(actions);
    }
    return card;
  }
}
