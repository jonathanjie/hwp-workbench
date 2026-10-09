import {t} from './i18n.js';
import {element} from './dom.js';

export class Toast {
  constructor(host) {
    this.host = host;
    this.host.onmouseenter = () => clearTimeout(this.timer);
    this.host.onmouseleave = () => this.schedule();
    this.host.onfocusin = () => clearTimeout(this.timer);
    this.host.onfocusout = () => this.schedule();
  }

  show(key, values = {}, action = null) {
    this.message = {key, values, action};
    this.render();
    this.schedule();
  }

  render() {
    if (!this.message) {
      return;
    }
    const {key, values, action} = this.message;
    const content = element('span', '', t(key, values));
    this.host.replaceChildren(content);
    this.host.hidden = false;
    if (action) {
      const button = element('button', 'btn ghost', t(action.label));
      button.onclick = () => {
        this.clear();
        action.run();
      };
      this.host.append(button);
    }
  }

  schedule() {
    clearTimeout(this.timer);
    if (this.message) {
      this.timer = setTimeout(() => this.clear(), 7000);
    }
  }

  clear() {
    clearTimeout(this.timer);
    this.message = null;
    this.host.hidden = true;
    this.host.replaceChildren();
  }
}
