const path = require('node:path');
const {validateDroppedFile} = require('./recent-store.cjs');

function openArgument(argv) {
  const flags = argv.flatMap((value, index) => value === '--open' ? [index] : []);
  if (flags.length !== 1) {
    return null;
  }
  const file = argv[flags[0] + 1];
  return typeof file === 'string' && path.isAbsolute(file) ? file : null;
}

// The second instance sends its own parsed request as lock data. Chromium can
// reorder the argv that `second-instance` receives (switches before paths), so
// argv is only a fallback.
function requestedOpen(argv, additionalData) {
  const file = additionalData?.open;
  if (typeof file === 'string' && path.isAbsolute(file)) {
    return file;
  }
  return openArgument(argv);
}

class OpenHandoff {
  constructor({hasStaged, confirm, load, complete, focus}) {
    this.hasStaged = hasStaged;
    this.confirm = confirm;
    this.load = load;
    this.complete = complete;
    this.focus = focus;
  }

  async open(file) {
    const granted = await validateDroppedFile(file);
    this.focus();
    if (this.hasStaged() && !(await this.confirm())) {
      return false;
    }
    const result = await this.load(granted);
    this.complete(result);
    this.focus();
    return true;
  }
}

module.exports = {openArgument, requestedOpen, OpenHandoff};
