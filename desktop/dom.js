export function element(tag, className = '', text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

export function highlightText(text, start, end) {
  const fragment = document.createDocumentFragment();
  fragment.append(document.createTextNode(text.slice(0, start)), element('mark', '', text.slice(start, end)), document.createTextNode(text.slice(end)));
  return fragment;
}
