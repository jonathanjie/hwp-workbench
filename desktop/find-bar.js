import {t} from './i18n.js';
import {findMatches, replaceMatches} from './search.js';
import {element} from './dom.js';

export class FindBar {
  constructor(host, getModel, {onHighlight, onLocate, onStaged, location, beforeReplace}) {
    this.host = host;
    this.getModel = getModel;
    this.onHighlight = onHighlight;
    this.onLocate = onLocate;
    this.onStaged = onStaged;
    this.location = location;
    this.beforeReplace = beforeReplace;
    this.query = host.querySelector('#findQuery');
    this.replacement = host.querySelector('#replaceQuery');
    this.caseToggle = host.querySelector('#findCase');
    this.active = 0;
    this.caseSensitive = false;
    this.query.oninput = () => {
      this.active = 0;
      this.refresh(true);
    };
    this.caseToggle.onclick = () => {
      this.caseSensitive = !this.caseSensitive;
      this.caseToggle.setAttribute('aria-pressed', String(this.caseSensitive));
      this.refresh(true);
    };
    host.querySelector('#findPrevious').onclick = () => this.move(-1);
    host.querySelector('#findNext').onclick = () => this.move(1);
    host.querySelector('#findClose').onclick = () => this.close();
    host.querySelector('#findReplaceToggle').onclick = () => this.toggleReplace();
    host.querySelector('#replaceOne').onclick = () => this.replace(false);
    host.querySelector('#replaceAll').onclick = () => this.replace(true);
    host.onkeydown = event => {
      if (event.isComposing) {
        return;
      }
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        this.close();
      } else if (event.key === 'Enter' && event.target === this.query) {
        event.preventDefault();
        this.move(event.shiftKey ? -1 : 1);
      }
    };
  }

  open(replace = false, query) {
    if (this.host.hidden) {
      this.returnFocus = document.activeElement;
    }
    this.host.hidden = false;
    this.toggleReplace(replace);
    if (query !== undefined) {
      this.query.value = query;
    }
    this.refresh();
    this.query.focus();
    this.query.select();
  }

  toggleReplace(show = this.host.querySelector('#replaceRow').hidden) {
    this.host.querySelector('#replaceRow').hidden = !show;
    this.host.querySelector('#findReplaceToggle').setAttribute('aria-expanded', String(show));
  }

  close() {
    this.host.hidden = true;
    this.host.querySelector('#skippedPopover').hidePopover();
    this.onHighlight([], -1);
    this.returnFocus?.focus({preventScroll: true});
  }

  refresh(locate = false) {
    if (this.host.hidden) {
      return;
    }
    this.matches = findMatches(this.getModel(), this.query.value, this.caseSensitive);
    this.active = Math.min(this.active, Math.max(0, this.matches.length - 1));
    this.host.querySelector('#findCount').textContent = `${this.matches.length ? this.active + 1 : 0} / ${this.matches.length}`;
    for (const id of ['replaceOne', 'replaceAll', 'findPrevious', 'findNext']) {
      this.host.querySelector(`#${id}`).disabled = !this.matches.length;
    }
    this.onHighlight(this.matches, this.active);
    if (locate && this.matches.length) {
      this.onLocate(this.matches[this.active]);
    }
  }

  move(direction) {
    if (!this.matches?.length) {
      return;
    }
    this.active = (this.active + direction + this.matches.length) % this.matches.length;
    this.refresh(true);
  }

  replace(all) {
    if (!this.beforeReplace()) {
      return;
    }
    const model = this.getModel();
    if (!model || !this.matches?.length) {
      return;
    }
    const selected = all ? this.matches : [this.matches[this.active]];
    const result = replaceMatches(model, selected, this.replacement.value);
    const skipped = this.host.querySelector('#skippedPopover');
    skipped.replaceChildren(...result.skipped.map(match => element('div', 'skipped-match', `${this.location(match.node)} · ${t(match.reason)}`)));
    const button = this.host.querySelector('#skippedButton');
    button.hidden = !result.skipped.length;
    button.textContent = t('skipped', {n: result.skipped.length});
    if (result.replaced) {
      this.onStaged(result.replaced, result.paragraphs);
    } else if (result.skipped.length) {
      this.move(1);
    }
    this.refresh(true);
    if (result.skipped.length) {
      skipped.showPopover();
    }
  }
}
