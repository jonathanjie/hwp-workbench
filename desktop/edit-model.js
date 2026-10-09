export function textError(text, original, format) {
  if ([...text].length > 200000) {
    return 'tooLong';
  }
  for (const character of text) {
    const code = character.codePointAt(0);
    if (code < 32 || (code >= 0xd800 && code <= 0xdfff) || code === 0xfffe || code === 0xffff) {
      return 'controls';
    }
  }
  if (format === 'hwp' && text.length !== original.length) {
    return 'lengthError';
  }
  return null;
}

export class EditModel {
  constructor(document) {
    this.document = document;
    this.nodes = new Map(document.nodes.map(node => [node.id, node]));
    this.edits = {};
    this.history = [];
    this.future = [];
  }

  current(node) {
    return Object.hasOwn(this.edits, node.id) ? this.edits[node.id] : node.text;
  }

  stage(node, text) {
    if (!node.editable || textError(text, node.text, this.document.format) || text === this.current(node)) {
      return false;
    }
    this.history.push({...this.edits});
    this.future = [];
    if (text === node.text) {
      delete this.edits[node.id];
    } else {
      this.edits[node.id] = text;
    }
    return true;
  }

  revert(id) {
    if (Object.hasOwn(this.edits, id)) {
      this.history.push({...this.edits});
      this.future = [];
      delete this.edits[id];
    }
  }

  undo() {
    if (this.history.length) {
      this.future.push({...this.edits});
      this.edits = this.history.pop();
    }
  }

  redo() {
    if (this.future.length) {
      this.history.push({...this.edits});
      this.edits = this.future.pop();
    }
  }

  restore(draft) {
    if (draft?.schemaVersion !== 1 || draft.baseSha256 !== this.document.sha256 || !Array.isArray(draft.operations)) {
      return;
    }
    for (const operation of draft.operations) {
      const node = this.nodes.get(operation.nodeId);
      if (operation.op === 'set_text' && node?.editable && operation.expectedText === node.text && typeof operation.text === 'string' &&
          !textError(operation.text, node.text, this.document.format)) {
        this.stage(node, operation.text);
      }
    }
  }

  patch() {
    return {
      schemaVersion: 1,
      baseSha256: this.document.sha256,
      operations: Object.entries(this.edits).map(([nodeId, text]) => ({
        op: 'set_text', nodeId, expectedText: this.nodes.get(nodeId).text, text
      }))
    };
  }
}
