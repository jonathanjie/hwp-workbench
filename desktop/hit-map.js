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

function nodeParagraphs(nodes) {
  const groups = new Map();
  for (const node of nodes) {
    const section = sectionIndex(node);
    const key = `${section}:${node.paragraph ?? node.id}`;
    if (!groups.has(key)) {
      groups.set(key, {section, nodes: [], text: ''});
    }
    const group = groups.get(key);
    group.nodes.push(node);
    group.text += node.text;
  }
  return [...groups.values()];
}

export function mapTextRuns(runs, nodes, documentRuns = runs) {
  const paragraphs = nodeParagraphs(nodes);
  const assignments = new Map();
  const positions = new Map();
  // Table paraIdx values are local to a cell; page-run order is the common
  // document-order reference until native anchors are supplied.
  const groups = paragraphBounds(documentRuns);
  for (const group of groups) {
    const section = group.runs[0].secIdx;
    const uniqueRuns = new Map();
    for (const run of group.runs) {
      uniqueRuns.set(`${run.charStart}:${run.text}`, run);
    }
    const ordered = [...uniqueRuns.values()].sort((a, b) => a.charStart - b.charStart);
    const text = ordered.map(run => run.text).join('');
    if (!text) {
      continue;
    }
    const start = positions.get(section) ?? 0;
    const index = paragraphs.findIndex((paragraph, index) => index >= start && paragraph.section === section && paragraph.text.includes(text));
    if (index !== -1) {
      assignments.set(group.key, paragraphs[index]);
      positions.set(section, index + 1);
    }
  }
  return runs.map(run => {
    if (!run.text || !Number.isInteger(run.secIdx) || !Number.isInteger(run.paraIdx)) {
      return {run, node: null};
    }
    const paragraphIndex = run.parentParaIdx ?? run.paraIdx;
    const inCell = run.inCell ?? (run.cellIdx !== undefined || run.cellPath !== undefined);
    const anchored = nodes.filter(node => node.anchor?.section === run.secIdx && node.anchor.paragraph === paragraphIndex &&
      node.anchor.inCell === inCell &&
      run.charStart >= node.anchor.charOffset &&
      run.charStart + run.text.length <= node.anchor.charOffset + node.text.length &&
      node.text.slice(run.charStart - node.anchor.charOffset, run.charStart - node.anchor.charOffset + run.text.length) === run.text);
    if (anchored.length) {
      return {run, node: anchored.length === 1 ? anchored[0] : null};
    }
    // Anchored nodes never fall back to a weaker text-only identity.
    const paragraph = assignments.get(paragraphKey(run));
    let offset = 0;
    const candidates = [];
    for (const node of paragraph?.nodes || []) {
      if (!node.anchor && node.text.includes(run.text)) {
        candidates.push({node, offset});
      }
      offset += node.text.length;
    }
    const exact = candidates.filter(candidate => candidate.node.text.slice(run.charStart - candidate.offset,
      run.charStart - candidate.offset + run.text.length) === run.text);
    const node = exact.length === 1 ? exact[0].node : candidates.length === 1 ? candidates[0].node : null;
    return {run, node};
  });
}
