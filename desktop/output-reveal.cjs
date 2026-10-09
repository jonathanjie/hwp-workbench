class OutputReveal {
  constructor(reveal) {
    this.reveal = reveal;
    this.clear();
  }

  saved(file) {
    this.output = file;
  }

  clear() {
    this.output = null;
  }

  show(...args) {
    if (args.length) {
      throw Error('Reveal accepts no arguments');
    }
    if (!this.output) {
      throw Error('No saved copy');
    }
    this.reveal(this.output);
    return true;
  }
}

module.exports = {OutputReveal};
