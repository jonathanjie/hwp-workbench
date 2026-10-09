// Single shortcut table. Match physical codes, not the current input alphabet.
// Windows Ctrl expands to Control and Command; system-owned keys are excluded.
// Read-only format commands are bound; format mutation commands stay unbound.
export const KEYMAP = [
  {id: 'open', mac: ['Meta+KeyO'], windows: ['Alt+KeyO', 'Ctrl+KeyO']},
  {id: 'save', mac: ['Meta+KeyS'], windows: ['Alt+KeyS', 'Ctrl+KeyS']},
  {id: 'saveCopy', mac: ['Meta+Shift+KeyS'], windows: ['Alt+KeyV', 'Ctrl+Shift+KeyS']},
  {id: 'undo', mac: ['Meta+KeyZ'], windows: ['Ctrl+KeyZ']},
  {id: 'redo', mac: ['Meta+Shift+KeyZ'], windows: ['Ctrl+Shift+KeyZ']},
  {id: 'selectAll', mac: ['Meta+KeyA'], windows: ['Ctrl+KeyA']},
  {id: 'find', mac: ['Meta+KeyF'], windows: ['Ctrl+KeyF']},
  {id: 'replace', mac: ['Meta+Alt+KeyF'], windows: ['Ctrl+KeyH', 'Meta+Alt+KeyF']},
  {id: 'charFormat', mac: ['Alt+KeyL'], windows: ['Alt+KeyL']},
  {id: 'paraFormat', mac: ['Alt+KeyT'], windows: ['Alt+KeyT']},
  {id: 'pageFormat', mac: ['F7'], windows: ['F7']},
  {id: 'styleFormat', mac: ['F6'], windows: ['F6']},
  {id: 'symbols', mac: ['Meta+F10'], windows: ['Ctrl+F10']},
  {id: 'controlMarks', mac: ['Meta+KeyG KeyC'], windows: ['Ctrl+KeyG KeyC']},
  {id: 'cellSlip', mac: ['F5'], windows: ['F5']},
  {id: 'nextCell', mac: ['Tab'], windows: ['Tab']},
  {id: 'previousCell', mac: ['Shift+Tab'], windows: ['Shift+Tab']},
  {id: 'palette', mac: ['Meta+KeyK'], windows: ['Ctrl+KeyK']},
  {id: 'shortcuts', mac: ['Meta+Slash'], windows: ['Ctrl+Slash']},
  {id: 'togglePanel', mac: ['Meta+Alt+KeyI'], windows: ['Meta+Alt+KeyI']},
  {id: 'zoomIn', mac: ['Meta+Equal', 'Meta+Shift+Equal'], windows: ['Ctrl+Equal', 'Ctrl+Shift+Equal']},
  {id: 'zoomOut', mac: ['Meta+Minus'], windows: ['Ctrl+Minus']},
  {id: 'fitWidth', mac: ['Meta+Digit0'], windows: ['Ctrl+Digit0']}
];

const MODIFIERS = ['Ctrl', 'Meta', 'Alt', 'Shift'];
const SYSTEM = new Set(['Ctrl+Space', 'Meta+KeyQ', 'Meta+KeyW', 'Meta+KeyH', 'Meta+Tab', 'Meta+Space']);
function canonical(key) {
  const parts = key.split('+');
  const code = parts.pop();
  return [...MODIFIERS.filter(mod => parts.includes(mod)), code].join('+');
}
function systemKey(key) {
  const parts = key.split('+');
  const code = parts.at(-1);
  return SYSTEM.has(key) || (parts.includes('Meta') && ['KeyQ', 'KeyW', 'KeyH', 'Tab', 'Space'].includes(code)) ||
    (parts.includes('Ctrl') && code === 'Space');
}
export function bindings(scheme, table = KEYMAP) {
  const rows = [];
  for (const command of table) {
    for (const key of command[scheme === 'mac' ? 'mac' : 'windows']) {
      const alternatives = key.includes('Ctrl+') && scheme !== 'mac' ? [key, key.replaceAll('Ctrl+', 'Meta+')] : [key];
      for (const alternative of alternatives) {
        const sequence = alternative.split(' ').map(canonical);
        if (!sequence.some(systemKey)) {
          rows.push({id: command.id, sequence});
        }
      }
    }
  }
  return rows;
}
export function validateKeymap(table) {
  const errors = [];
  const ids = new Set();
  for (const row of table) {
    if (ids.has(row.id)) {
      errors.push(`Duplicate command: ${row.id}`);
    }
    ids.add(row.id);
    for (const key of [...row.mac, ...row.windows]) {
      const repeated = key.split(' ').some(stroke => {
        const modifiers = stroke.split('+').slice(0, -1);
        return new Set(modifiers).size !== modifiers.length;
      });
      if (repeated || !/^(?:(?:Ctrl|Meta|Alt|Shift)\+)*(?:Key[A-Z]|Digit[0-9]|F(?:[1-9]|1[0-2])|Tab|Slash|Equal|Minus)(?: (?:(?:Ctrl|Meta|Alt|Shift)\+)*(?:Key[A-Z]))?$/.test(key)) {
        errors.push(`Invalid key: ${key}`);
      }
    }
  }
  for (const scheme of ['mac', 'windows']) {
    const seen = new Set();
    for (const row of bindings(scheme, table)) {
      const key = row.sequence.join(' ');
      if (seen.has(key)) {
        errors.push(`Duplicate ${scheme}: ${key}`);
      }
      seen.add(key);
    }
  }
  return errors;
}
export function keyLabel(key, windows = false) {
  return key.split(' ').map(stroke => stroke.split('+').map(part => {
    if (windows) {
      return part.replace('Meta', '⌘').replace('Key', '').replace('Digit', '');
    }
    return ({Ctrl: '⌃', Meta: '⌘', Alt: '⌥', Shift: '⇧', Tab: '⇥', Slash: '/', Equal: '+', Minus: '−'}[part] || part.replace('Key', '').replace('Digit', ''));
  }).join(windows ? '+' : '')).join(' → ');
}
export function shortcutFor(id, scheme) {
  return bindings(scheme).filter(row => row.id === id).map(row => keyLabel(row.sequence.join(' '))).join(' / ');
}
export function acceleratorFor(id, scheme) {
  const row = bindings(scheme).find(row => row.id === id && row.sequence.length === 1);
  if (!row) {
    return undefined;
  }
  return row.sequence[0].replace('Ctrl', 'Control').replace('Meta', 'Command').replace('Key', '').replace('Digit', '').replace('Equal', '=').replace('Minus', '-').replace('Slash', '/');
}
export class Keymap {
  constructor({scheme = 'windows', onPending = () => {}, setTimer = setTimeout, clearTimer = clearTimeout} = {}) {
    this.scheme = scheme;
    this.onPending = onPending;
    // Call the timer functions unbound: browsers throw "Illegal invocation"
    // when setTimeout runs with an object other than window as `this`.
    this.startTimer = callback => setTimer(callback, 1500);
    this.stopTimer = timer => clearTimer(timer);
  }
  cancel() {
    this.stopTimer(this.timer);
    this.pending = null;
    this.onPending('');
  }
  match(event) {
    if (event.isComposing) {
      return null;
    }
    if (this.pending && event.code === 'Escape') {
      this.cancel();
      return 'cancelChord';
    }
    const key = [event.ctrlKey && 'Ctrl', event.metaKey && 'Meta', event.altKey && 'Alt', event.shiftKey && 'Shift', event.code].filter(Boolean).join('+');
    if (systemKey(key)) {
      return null;
    }
    const rows = bindings(this.scheme);
    if (this.pending) {
      const first = this.pending;
      this.cancel();
      const chord = rows.find(row => row.sequence.length === 2 && row.sequence[0] === first && row.sequence[1] === key);
      if (chord) {
        return chord.id;
      }
    }
    const row = rows.find(row => row.sequence[0] === key);
    if (!row) {
      return null;
    }
    if (row.sequence.length === 2) {
      this.pending = key;
      this.onPending(keyLabel(key));
      this.timer = this.startTimer(() => this.cancel());
      return 'pending';
    }
    return row.id;
  }
}
