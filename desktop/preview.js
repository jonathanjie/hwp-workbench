import {nativeLength} from './native-text.js';
import {wordDiffSegments} from './changes.js';

export function previewNodes(nodes, edits) {
  return nodes.map(node => {
    let anchor = node.anchor;
    if (Number.isInteger(anchor?.charOffset)) {
      const shift = nodes.filter(previous => previous.anchor?.section === anchor.section &&
        previous.paragraph === node.paragraph && Number.isInteger(previous.anchor.charOffset) && previous.anchor.charOffset < anchor.charOffset)
        .reduce((total, previous) => total + nativeLength(edits[previous.id] ?? previous.text) - nativeLength(previous.text), 0);
      anchor = {...anchor, charOffset: anchor.charOffset + shift};
    }
    return {...node, anchor, text: edits[node.id] ?? node.text};
  });
}

export function changedRanges(before, after) {
  const ranges = [];
  let offset = 0;
  let removed = false;
  for (const segment of wordDiffSegments(before, after)) {
    if (segment.type === 'del') {
      removed = true;
      continue;
    }
    if (segment.type === 'ins') {
      ranges.push({start: offset, end: offset + segment.text.length});
      removed = false;
    } else if (removed) {
      ranges.push({start: offset, end: offset});
      removed = false;
    }
    offset += segment.text.length;
  }
  if (removed) {
    ranges.push({start: offset, end: offset});
  }
  return ranges;
}

// Retain one previous native view so lazy page or image failures can roll back.
// A later successful preview releases the older snapshot, bounding retention.
export function commitPagePreview(previous, next, {pages, thumbnails, onMount, onRollback}) {
  const oldPages = [...pages.children];
  const oldThumbnails = [...thumbnails.children];
  const newPages = [...next.pages.children];
  const newThumbnails = [...next.thumbnails.children];
  previous.suspend();
  previous.releasePrevious?.();
  previous.releasePrevious = null;
  let rolledBack = false;

  function mount(view, pageNodes, thumbnailNodes) {
    pages.replaceChildren(...pageNodes);
    thumbnails.replaceChildren(...thumbnailNodes);
    view.pages = pages;
    view.thumbnails = thumbnails;
    onMount(view);
    view.activate();
  }

  next.releasePrevious = () => previous.clear(false);
  next.onFailure = error => {
    if (rolledBack) {
      return;
    }
    rolledBack = true;
    next.releasePrevious = null;
    next.clear(false);
    mount(previous, oldPages, oldThumbnails);
    onRollback(error);
  };
  try {
    mount(next, newPages, newThumbnails);
  } catch (error) {
    next.onFailure(error);
  }
}
