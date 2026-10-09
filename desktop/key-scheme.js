export class KeySchemeSelector {
  constructor(group, onChange) {
    this.buttons = [...group.querySelectorAll('[role="radio"]')];
    this.onChange = onChange;
    this.buttons.forEach((button, index) => {
      button.onclick = () => this.select(index);
      button.onkeydown = event => this.keydown(event, index);
    });
  }

  setScheme(scheme) {
    for (const button of this.buttons) {
      const selected = button.dataset.scheme === scheme;
      button.setAttribute('aria-checked', String(selected));
      button.tabIndex = selected ? 0 : -1;
      button.classList.toggle('on', selected);
    }
  }

  select(index) {
    const button = this.buttons[index];
    this.setScheme(button.dataset.scheme);
    this.onChange(button.dataset.scheme);
  }

  keydown(event, index) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
      return;
    }
    const count = this.buttons.length;
    let next;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        next = (index + 1) % count;
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        next = (index + count - 1) % count;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = count - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    this.select(next);
    this.buttons[next].focus({preventScroll: true});
  }
}
