import {t} from './i18n.js';

export function renderStructure(documentData, model, parent, onSelect) {
  const nodes = model.nodes;
  function render(view) {
    if (view.type === 'text') {
      const node = nodes.get(view.node);
      const span = document.createElement('span');
      if (!node) {
        return span;
      }
      span.className = 'text-node';
      span.classList.toggle('edited', Object.hasOwn(model.edits, node.id));
      span.textContent = model.current(node) || t('empty');
      span.dataset.nodeId = node.id;
      span.tabIndex = 0;
      span.setAttribute('role', 'button');
      if (node.style?.bold) {
        span.style.fontWeight = '700';
      }
      if (node.style?.italic) {
        span.style.fontStyle = 'italic';
      }
      if (node.style?.size) {
        span.style.fontSize = `${node.style.size / 10}em`;
      }
      const select = () => {
        const page = span.closest('.page');
        const bounds = span.getBoundingClientRect();
        const paper = page.getBoundingClientRect();
        onSelect(node, {page, bounds: {x: bounds.left - paper.left, y: bounds.top - paper.top, w: bounds.width, h: bounds.height}, width: paper.width, height: paper.height, focus: span, paragraph: node.anchor?.paragraph ?? Math.max(0, (node.paragraph || 1) - 1)});
      };
      span.onclick = select;
      span.onkeydown = event => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          select();
        }
      };
      return span;
    }
    if (view.type === 'opaque') {
      const placeholder = document.createElement('span');
      placeholder.className = 'opaque';
      placeholder.textContent = t('preserved');
      return placeholder;
    }
    const tags = {section: 'article', paragraph: 'div', table: 'table', row: 'tr', cell: 'td'};
    const element = document.createElement(tags[view.type] || 'div');
    element.className = view.type === 'section' ? 'page structure-page' : view.type === 'table' ? 'doc-table' : view.type;
    if (view.type === 'section') {
      element.tabIndex = -1;
    }
    if (view.type === 'cell') {
      element.colSpan = view.colspan || 1;
      element.rowSpan = view.rowspan || 1;
    }
    for (const child of view.children || []) {
      element.append(render(child));
    }
    return element;
  }
  parent.replaceChildren(...documentData.view.map(render));
}
