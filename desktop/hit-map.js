import {nativeLength, nativeSlice, readerText} from './native-text.js';

export function sectionIndex(node) {
  if (Number.isInteger(node.anchor?.section)) {
    return node.anchor.section;
  }
  const match = /(?:section|Section)(\d+)/.exec(node.part || '');
  return match ? Number(match[1]) : null;
}

export function paragraphKey(run) {
  const cell = run.cellAddr ?? run.cellPath ?? run.cellIdx ?? '';
  return `${run.secIdx}:${run.parentParaIdx ?? ''}:${run.paraIdx}:${run.cellParaIdx ?? ''}:${JSON.stringify(cell)}`;
}

export function paragraphBounds(runs) {
  const groups = new Map();
  for (const run of runs) {
    // Bullets and some opaque controls have no native paragraph identity.
    if (!Number.isInteger(run.secIdx) || !Number.isInteger(run.paraIdx)) {
      continue;
    }
    const key = paragraphKey(run);
    if (!groups.has(key)) {
      groups.set(key, {key, runs: []});
    }
    groups.get(key).runs.push(run);
  }
  return [...groups.values()].map(group => {
    const x = Math.min(...group.runs.map(run => run.x));
    const y = Math.min(...group.runs.map(run => run.y));
    const right = Math.max(...group.runs.map(run => run.x + run.w));
    const bottom = Math.max(...group.runs.map(run => run.y + run.h));
    return {...group, bounds: {x, y, w: right - x, h: bottom - y}};
  });
}

function runKey(run) {
  return `${paragraphKey(run)}:${run.charStart}:${run.text}`;
}

function isNestedRun(run) {
  return Number.isInteger(run.parentParaIdx) || run.cellPath !== undefined || run.cellIdx !== undefined || run.inCell === true;
}

// Native order inside one top-level paragraph: control, cell and cell
// paragraph, at each nesting level. Layout can draw floating boxes in another order.
function cellOrder(run) {
  const path = Array.isArray(run.cellPath) && run.cellPath.every(step => typeof step === 'object')
    ? run.cellPath
    : [{controlIndex: run.controlIdx, cellIndex: run.cellIdx, cellParaIndex: run.cellParaIdx ?? run.paraIdx}];
  return path.flatMap(step => [step.controlIndex ?? 0, step.cellIndex ?? 0, step.cellParaIndex ?? 0]);
}

function compareNativeOrder(a, b) {
  const left = cellOrder(a);
  const right = cellOrder(b);
  for (let index = 0; index < Math.min(left.length, right.length); index += 1) {
    if (left[index] !== right[index]) {
      return left[index] - right[index];
    }
  }
  // An enclosing cell paragraph sorts before a table nested inside it.
  return left.length - right.length || (a.charStart ?? 0) - (b.charStart ?? 0);
}

// Nodes without a character offset (table cells, text boxes, or nodes with no
// anchor) are matched by text. Each scope holds the nodes of one top-level
// paragraph, so a match never leaves the table or text box that holds the run.
function unanchoredScopes(nodes, texts) {
  const scopes = new Map();
  for (const node of nodes) {
    if (Number.isInteger(node.anchor?.charOffset)) {
      continue;
    }
    const anchored = Number.isInteger(node.anchor?.paragraph);
    const key = `${sectionIndex(node)}:${anchored ? node.anchor.paragraph : 'loose'}`;
    if (!scopes.has(key)) {
      scopes.set(key, {nodes: [], text: '', runs: [], nativeOrder: anchored});
    }
    const scope = scopes.get(key);
    scope.nodes.push({node, start: scope.text.length});
    scope.text += texts.get(node);
  }
  return scopes;
}

// Within one scope, runs are walked in native order. Each run takes the next
// occurrence of its text and opens the node where that occurrence starts; a
// run can cross into the next node when both have the same character style.
function mapUnanchoredRuns(documentRuns, nodes, texts) {
  const scopes = unanchoredScopes(nodes, texts);
  for (const group of paragraphBounds(documentRuns)) {
    const first = group.runs[0];
    const nestedKey = `${first.secIdx}:${first.parentParaIdx ?? first.paraIdx}`;
    const scope = (isNestedRun(first) && scopes.get(nestedKey)) || scopes.get(`${first.secIdx}:loose`);
    if (!scope) {
      continue;
    }
    // A paragraph split across pages can repeat a run; keep one copy.
    const unique = new Map(group.runs.map(run => [`${run.charStart}:${run.text}`, run]));
    scope.runs.push(...[...unique.values()].sort((a, b) => a.charStart - b.charStart));
  }
  const mapped = new Map();
  for (const scope of scopes.values()) {
    // Without a top-level anchor, page order is the only document-order reference.
    if (scope.nativeOrder) {
      scope.runs.sort(compareNativeOrder);
    }
    let cursor = 0;
    for (const run of scope.runs) {
      if (!run.text) {
        continue;
      }
      const at = scope.text.indexOf(run.text, cursor);
      if (at === -1) {
        continue;
      }
      const owner = scope.nodes.findLast(item => item.start <= at && texts.get(item.node).length > 0);
      mapped.set(runKey(run), owner.node);
      // Spaces occur everywhere; they must not move the cursor past real text.
      if (run.text.trim()) {
        cursor = at + run.text.length;
      }
    }
  }
  return mapped;
}

// The page view maps each page against the same document runs and nodes.
const unanchoredCache = new WeakMap();

export function mapTextRuns(runs, nodes, documentRuns = runs) {
  const texts = new Map(nodes.map(node => [node, readerText(node)]));
  let cached = unanchoredCache.get(documentRuns);
  if (cached?.nodes !== nodes) {
    cached = {nodes, mapped: mapUnanchoredRuns(documentRuns, nodes, texts)};
    unanchoredCache.set(documentRuns, cached);
  }
  const unanchored = cached.mapped;
  return runs.map(run => {
    if (!Number.isInteger(run.secIdx) || !Number.isInteger(run.paraIdx)) {
      return {run, node: null};
    }
    const paragraphIndex = run.parentParaIdx ?? run.paraIdx;
    const inCell = run.inCell ?? (run.cellIdx !== undefined || run.cellPath !== undefined);
    const anchored = nodes.filter(node => (run.text || node.text === '') && node.anchor?.section === run.secIdx && node.anchor.paragraph === paragraphIndex &&
      node.anchor.inCell === inCell &&
      Number.isInteger(node.anchor.charOffset) && run.charStart >= node.anchor.charOffset &&
      run.charStart + nativeLength(run.text) <= node.anchor.charOffset + nativeLength(texts.get(node)) &&
      nativeSlice(texts.get(node), run.charStart - node.anchor.charOffset, nativeLength(run.text)) === run.text);
    if (anchored.length) {
      return {run, node: anchored.length === 1 ? anchored[0] : null};
    }
    // Nodes with a complete native text anchor never fall back to text matching.
    return {run, node: unanchored.get(runKey(run)) ?? null};
  });
}
