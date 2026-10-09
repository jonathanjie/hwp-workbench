function bounded(item) {
  return ['x', 'y', 'w', 'h'].every(key => Number.isFinite(item[key])) && item.w >= 0 && item.h >= 0;
}
export function controlMarks(controls, mapped, groups) {
  const marks = [];
  for (const control of controls) {
    if (!bounded(control)) {
      continue;
    }
    if (control.type === 'table') {
      marks.push({type: 'table', bounds: control});
      for (const cell of control.cells || []) {
        if (bounded(cell)) {
          marks.push({type: 'cell', bounds: cell});
        }
      }
    } else if (/field/i.test(control.type)) {
      marks.push({type: 'field', bounds: control});
    }
  }
  for (const {run, node} of mapped) {
    if (node && !node.editable && bounded(run)) {
      marks.push({type: 'locked', bounds: run});
    }
  }
  for (const group of groups) {
    const last = group.runs.at(-1);
    if (last && bounded(last)) {
      marks.push({type: 'paragraphEnd', bounds: {...last, x: last.x + last.w, w: 10}});
    }
  }
  return marks;
}

// Probe only the two ends of each addressed body run. Native text layout
// supplies field boundaries; cell offsets need a native cell path and stay
// unavailable when the renderer does not expose that path.
export function inlineFieldMarks(source, mapped) {
  const marks = [];
  for (const {run, node} of mapped) {
    if (!node?.anchor || node.anchor.inCell || !Number.isInteger(run.charStart) || !bounded(run)) {
      continue;
    }
    const offsets = [run.charStart, run.charStart + Math.max(0, [...run.text].length - 1)];
    try {
      if (offsets.some(offset => JSON.parse(source.getFieldInfoAt(run.secIdx, run.paraIdx, offset)).inField === true)) {
        marks.push({type: 'field', bounds: run});
      }
    } catch {
      // An unreadable native field has no inferred bounds.
    }
  }
  return marks;
}
