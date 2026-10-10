export function isInlineMarker(character) {
  const code = character.codePointAt(0);
  return code >= 0xfdd0 && code <= 0xfdef;
}

export function textError(text, original, lengthChangingText) {
  if ([...text].length > 200000) {
    return 'tooLong';
  }
  const markers = new Set([...original].filter(isInlineMarker));
  for (const character of text) {
    const code = character.codePointAt(0);
    if (code < 32 || (code >= 0xd800 && code <= 0xdfff) || code === 0xfffe || code === 0xffff) {
      return 'controls';
    }
    if (isInlineMarker(character) && !markers.delete(character)) {
      return 'inlineMarker';
    }
  }
  if (!lengthChangingText && text.length !== original.length) {
    return 'lengthError';
  }
  return null;
}

export class EditModel {
  constructor(document) {
    this.document = document;
    this.lengthChangingText = document.capabilities?.lengthChangingText === true;
    this.nodes = new Map(document.nodes.map(node => [node.id, node]));
    this.edits = {};
    this.origins = {};
    this.history = [];
    this.future = [];
  }

  current(node) {
    return Object.hasOwn(this.edits, node.id) ? this.edits[node.id] : node.text;
  }

  stage(node, text) {
    return this.stageBatch([{node, text}]);
  }

  stageBatch(changes) {
    const next = {...this.edits};
    const origins = {...this.origins};
    for (const {node, text, author} of changes) {
      if (this.nodes.get(node.id) !== node || !node.editable || textError(text, node.text, this.lengthChangingText)) {
        return false;
      }
      delete origins[node.id];
      if (text !== node.text && author) {
        origins[node.id] = author;
      }
      if (text === node.text) {
        delete next[node.id];
      } else {
        next[node.id] = text;
      }
    }
    if (Object.keys(next).length > 1000 || JSON.stringify(next) === JSON.stringify(this.edits)) {
      return false;
    }
    this.history.push(this.snapshot());
    this.future = [];
    this.edits = next;
    this.origins = origins;
    return true;
  }

  revert(id) {
    if (Object.hasOwn(this.edits, id)) {
      this.history.push(this.snapshot());
      this.future = [];
      delete this.edits[id];
      delete this.origins[id];
    }
  }

  undo() {
    if (this.history.length) {
      this.future.push(this.snapshot());
      this.restoreSnapshot(this.history.pop());
    }
  }

  redo() {
    if (this.future.length) {
      this.history.push(this.snapshot());
      this.restoreSnapshot(this.future.pop());
    }
  }

  snapshot() {
    return {edits: {...this.edits}, origins: {...this.origins}};
  }

  restoreSnapshot(snapshot) {
    this.edits = snapshot.edits;
    this.origins = snapshot.origins;
  }

  restore(draft) {
    if (draft?.schemaVersion !== 1 || draft.baseSha256 !== this.document.sha256 || !Array.isArray(draft.operations)) {
      return;
    }
    for (const operation of draft.operations) {
      const node = this.nodes.get(operation.nodeId);
      if (operation.op === 'set_text' && node?.editable && operation.expectedText === node.text && typeof operation.text === 'string' &&
          !textError(operation.text, node.text, this.lengthChangingText)) {
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
