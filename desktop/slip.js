import {t, roleLabel} from './i18n.js';
import {textError} from './edit-model.js';

function element(tag, className, text) {
  const result = document.createElement(tag);
  result.className = className;
  if (text !== undefined) {
    result.textContent = text;
  }
  return result;
}

export class ProofSlip {
  constructor(host, onStage, onRevert) {
    this.host = host;
    this.onStage = onStage;
    this.onRevert = onRevert;
  }

  get dirty() {
    return !!this.editor && this.editor.value !== this.initial;
  }

  open(node, value, format, location, lengthChangingText) {
    this.close(false);
    this.node = node;
    this.initial = value;
    this.format = format;
    this.lengthChangingText = lengthChangingText;
    this.location = location;
    this.slip = element('section', 'slip');
    this.slip.setAttribute('role', 'dialog');
    this.slip.setAttribute('aria-label', t('text'));
    const header = element('header', '');
    this.crumbs = element('span', 'crumbs');
    this.closeButton = element('button', 'btn ghost');
    this.closeButton.append(element('kbd', '', 'esc'));
    this.closeButton.onclick = () => this.close();
    header.append(this.crumbs, this.closeButton);
    const original = element('div', 'was', node.text || t('empty'));
    this.editor = element('textarea', 'now');
    this.editor.id = 'textEdit';
    this.editor.rows = 1;
    this.editor.value = value;
    this.editor.spellcheck = false;
    this.editor.disabled = !node.editable;
    this.error = element('div', 'edit-error');
    this.error.setAttribute('role', 'alert');
    const footer = element('footer', '');
    this.counter = element('span', 'count');
    this.revertButton = element('button', 'btn ghost');
    this.revertButton.onclick = () => {
      if (this.node.editable) {
        this.onRevert(this.node.id);
      }
      this.close();
    };
    this.stageButton = element('button', 'btn seal');
    this.stageButton.id = 'applyText';
    this.stageButton.onclick = () => this.stage();
    footer.append(this.counter, this.revertButton, this.stageButton);
    this.slip.append(header, original, this.editor, this.error, footer);
    this.host.append(this.slip);
    this.editor.oninput = () => this.update();
    this.slip.onkeydown = event => {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        this.close();
      } else if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && !event.isComposing) {
        event.preventDefault();
        this.stage();
      }
    };
    this.relabel();
    this.position();
    this.editor.focus({preventScroll: true});
  }

  position() {
    if (!this.slip || !this.location.page.isConnected) {
      return;
    }
    const canvas = this.host.parentElement;
    const page = this.location.page.getBoundingClientRect();
    const desk = canvas.getBoundingClientRect();
    const scale = page.width / this.location.width;
    const x = page.left - desk.left + canvas.scrollLeft + this.location.bounds.x * scale - 12;
    const y = page.top - desk.top + canvas.scrollTop + (this.location.bounds.y + this.location.bounds.h) * scale + 12;
    this.slip.style.left = `${Math.max(12, Math.min(x, canvas.scrollLeft + canvas.clientWidth - this.slip.offsetWidth - 12))}px`;
    this.slip.style.top = `${y}px`;
  }

  update() {
    if (!this.editor) {
      return;
    }
    this.editor.style.height = 'auto';
    this.editor.style.height = `${Math.min(220, this.editor.scrollHeight)}px`;
    const error = textError(this.editor.value, this.node.text, this.lengthChangingText);
    this.error.textContent = !this.node.editable ? t('locked') : error ? t(error) : '';
    this.counter.textContent = !this.lengthChangingText ? t('sameLength', {a: this.editor.value.length, b: this.node.text.length}) : `${this.editor.value.length} / ${this.node.text.length}`;
    this.stageButton.disabled = !!error || !this.node.editable || !this.dirty;
  }

  relabel() {
    if (!this.slip) {
      return;
    }
    this.crumbs.textContent = `P${this.location.paragraph + 1} · ${roleLabel(this.node.role)} · ${this.format.toUpperCase()}`;
    this.editor.setAttribute('aria-label', t('text'));
    this.closeButton.setAttribute('aria-label', t('close'));
    this.revertButton.textContent = t('revert');
    this.stageButton.replaceChildren(document.createTextNode(t('stage')), element('kbd', '', '⌘↵'));
    this.slip.setAttribute('aria-label', t('text'));
    this.update();
  }

  stage() {
    if (!this.stageButton.disabled && this.onStage(this.node, this.editor.value)) {
      this.close();
    }
  }

  close(returnFocus = true) {
    const focus = this.location?.focus;
    this.host.replaceChildren();
    this.slip = null;
    this.editor = null;
    if (returnFocus) {
      const target = focus?.isConnected ? focus : this.host.parentElement;
      target.focus({preventScroll: true});
    }
  }
}
