import {t, labelFor, currentLanguage} from './i18n.js';
import {matchingCommands} from './commands.js';
import {element, highlightText} from './dom.js';

export class CommandPalette {
  constructor(dialog, commands, textResults, onText) {
    this.dialog = dialog;
    this.commands = commands;
    this.textResults = textResults;
    this.onText = onText;
    this.scope = 0;
    this.selected = 0;
    this.query = dialog.querySelector('input');
    this.list = dialog.querySelector('.results');
    this.scopes = dialog.querySelector('.scope');
    this.query.oninput = () => {
      this.selected = 0;
      this.render();
    };
    dialog.onkeydown = event => this.keydown(event);
    dialog.addEventListener('cancel', () => { this.restoreFocus = true; });
    dialog.addEventListener('close', () => {
      if (this.restoreFocus) {
        this.returnFocus?.focus({preventScroll: true});
      }
    });
    dialog.addEventListener('click', event => {
      if (event.target === dialog && event.clientY > dialog.getBoundingClientRect().bottom) {
        this.close();
      }
    });
  }

  open(query = '') {
    this.restoreFocus = true;
    if (!this.dialog.open) {
      this.returnFocus = document.activeElement;
      this.dialog.showModal();
    }
    this.query.value = query;
    this.scope = 0;
    this.selected = 0;
    this.render();
    this.query.focus();
  }

  close(restoreFocus = true) {
    this.restoreFocus = restoreFocus;
    this.dialog.close();
  }

  render() {
    this.query.setAttribute('aria-label', t('paletteQuery'));
    this.query.placeholder = t('paletteQuery');
    this.scopes.replaceChildren(...['all', 'commands', 'text'].map((key, index) => {
      const button = element('button', index === this.scope ? 'on' : '', t(key));
      button.type = 'button';
      button.setAttribute('aria-pressed', String(index === this.scope));
      button.onclick = () => {
        this.scope = index;
        this.selected = 0;
        this.render();
        this.query.focus();
      };
      return button;
    }));
    const query = this.query.value;
    const commands = this.scope === 2 ? [] : matchingCommands(this.commands.map(command => ({
      ...command, en: labelFor('en', command.label), ko: labelFor('ko', command.label)
    })), query);
    const matches = this.scope === 1 || !query ? [] : this.textResults(query);
    this.items = [...commands.map(command => ({command})), ...matches.map(match => ({match}))];
    this.selected = Math.max(0, Math.min(this.selected, this.items.length - 1));
    const groups = [];
    for (const [label, results] of [['commandGroup', commands], ['documentGroup', matches]]) {
      if (!results.length) {
        continue;
      }
      const group = element('div', 'grp');
      group.append(element('div', 'lbl', t(label)));
      for (const result of results) {
        const index = label === 'commandGroup' ? commands.indexOf(result) : commands.length + matches.indexOf(result);
        const row = element('button', `item${index === this.selected ? ' on' : ''}`);
        row.id = `palette-result-${index}`;
        row.type = 'button';
        row.setAttribute('role', 'option');
        row.setAttribute('aria-selected', String(index === this.selected));
        if (label === 'commandGroup') {
          row.disabled = result.enabled?.() === false;
          row.append(element('span', '', labelFor(currentLanguage(), result.label)));
          if (result.shortcut) {
            const keys = element('span', 'k');
            keys.append(element('kbd', '', result.shortcut));
            row.append(keys);
          }
        } else {
          row.append(element('span', 'loc', result.location));
          const context = element('span', 'ctx');
          const start = Math.max(0, result.start - 24);
          const text = result.text.slice(start, result.end + 60);
          context.append(highlightText(text, result.start - start, result.end - start));
          row.append(context);
        }
        row.onclick = () => this.run(index);
        group.append(row);
      }
      groups.push(group);
    }
    if (!groups.length) {
      groups.push(element('div', 'empty-results', t('noResults')));
    }
    this.list.replaceChildren(...groups);
    this.query.setAttribute('aria-activedescendant', this.items.length ? `palette-result-${this.selected}` : '');
  }

  run(index = this.selected) {
    const item = this.items[index];
    if (!item || item.command?.enabled?.() === false) {
      return;
    }
    this.close(false);
    if (item.command) {
      item.command.run(this.query.value);
    } else {
      this.onText(item.match);
    }
  }

  keydown(event) {
    if (event.isComposing) {
      return;
    }
    if (event.key === 'Tab') {
      event.preventDefault();
      this.scope = (this.scope + (event.shiftKey ? 2 : 1)) % 3;
      this.selected = 0;
      this.render();
      this.query.focus();
    } else if (['ArrowDown', 'ArrowUp'].includes(event.key)) {
      event.preventDefault();
      this.selected = (this.selected + (event.key === 'ArrowDown' ? 1 : this.items.length - 1)) % Math.max(1, this.items.length);
      this.render();
      document.getElementById(`palette-result-${this.selected}`)?.scrollIntoView({block: 'nearest'});
    } else if (event.key === 'Enter') {
      event.preventDefault();
      this.run();
    }
  }
}
