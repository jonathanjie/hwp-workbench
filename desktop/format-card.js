import {element} from './dom.js';
import {t} from './i18n.js';

function read(source, method, ...args) {
  try {
    return JSON.parse(source?.[method](...args));
  } catch {
    return {};
  }
}
export function readRhwpFormat(source, node, caretOffset = 0) {
  const anchor = node?.anchor;
  const result = {character: {}, paragraph: {}, page: {}, style: {}};
  if (!source || !anchor || anchor.inCell || anchor.charOffset === null) {
    return result;
  }
  const {section, paragraph, charOffset} = anchor;
  const char = read(source, 'getCharPropertiesAt', section, paragraph, charOffset + caretOffset);
  result.character = {font: char.fontFamily, size: Number.isFinite(char.fontSize) ? char.fontSize / 100 : undefined,
    widthRatio: char.ratios?.[0], letterSpacing: char.spacings?.[0], bold: char.bold, italic: char.italic,
    underline: char.underline, colour: char.textColor};
  const para = read(source, 'getParaPropertiesAt', section, paragraph);
  result.paragraph = {alignment: para.alignment,
    lineSpacing: para.lineSpacingType === 'Percent' && Number.isFinite(para.lineSpacing) ? `${para.lineSpacing}%` : undefined};
  const page = read(source, 'getPageDef', section);
  for (const [key, native] of Object.entries({width: 'width', height: 'height', left: 'marginLeft', right: 'marginRight',
    top: 'marginTop', bottom: 'marginBottom', header: 'marginHeader', footer: 'marginFooter', gutter: 'marginGutter'})) {
    if (Number.isFinite(page[native])) {
      result.page[key] = page[native] / 7200 * 25.4;
    }
  }
  result.style = read(source, 'getStyleAt', section, paragraph);
  return result;
}
export function mergeFormats(native, fallback) {
  const result = {};
  for (const group of ['character', 'paragraph', 'page', 'style']) {
    result[group] = {...fallback?.[group]};
    for (const [key, value] of Object.entries(native[group] || {})) {
      if (value !== null && value !== undefined) {
        result[group][key] = value;
      }
    }
  }
  return result;
}
const FIELDS = {
  charFormat: ['character', ['font', 'size', 'widthRatio', 'letterSpacing', 'bold', 'italic', 'underline', 'colour']],
  paraFormat: ['paragraph', ['alignment', 'left', 'right', 'indent', 'lineSpacing', 'before', 'after']],
  pageFormat: ['page', ['width', 'height', 'orientation', 'left', 'right', 'top', 'bottom', 'header', 'footer', 'gutter']],
  styleFormat: ['style', ['name']]
};
export function formatRows(kind, data) {
  const [group, keys] = FIELDS[kind];
  return keys.map(key => ({key, value: data[group]?.[key] ?? '—'}));
}
export function formatValue(kind, key, value, translate = t) {
  const unit = kind === 'pageFormat' && !['orientation'].includes(key) ? ' mm' :
    kind === 'charFormat' && key === 'size' ? ' pt' :
    kind === 'paraFormat' && ['left', 'right', 'indent', 'before', 'after'].includes(key) ? ' pt' : '';
  const enumKeys = {LEFT: 'left', left: 'left', RIGHT: 'right', right: 'right', CENTER: 'center', center: 'center', JUSTIFY: 'justify', justify: 'justify', portrait: 'portrait', landscape: 'landscape'};
  const enumKey = Object.hasOwn(enumKeys, value) ? enumKeys[value] : null;
  return typeof value === 'boolean' ? translate(value ? 'on' : 'formatOff') :
    typeof value === 'number' ? `${Number(value.toFixed(2))}${unit}` :
    ['alignment', 'orientation'].includes(key) && enumKey ? translate(enumKey) : String(value);
}
export function formatCardPosition(card, viewport, slip) {
  // Keep cards clear of the window band and leave a 12 px edge/gap.
  const gap = 12;
  const minimumTop = 58;
  const maximumLeft = Math.max(gap, viewport.width - card.width - gap);
  const maximumTop = Math.max(minimumTop, viewport.height - card.height - gap);
  let left = (viewport.width - card.width) / 2;
  let top = (viewport.height - card.height) / 2;
  if (slip) {
    top = slip.top;
    if (slip.right + gap + card.width <= viewport.width - gap) {
      left = slip.right + gap;
    } else if (slip.left - gap - card.width >= gap) {
      left = slip.left - gap - card.width;
    } else {
      left = slip.left;
      top = slip.bottom + gap + card.height <= viewport.height - gap ?
        slip.bottom + gap : slip.top - gap - card.height;
    }
  }
  return {
    left: Math.max(gap, Math.min(left, maximumLeft)),
    top: Math.max(minimumTop, Math.min(top, maximumTop))
  };
}

export function showFormat(dialog, kind, data, slip) {
  const focus = document.activeElement;
  const header = element('header');
  header.append(element('strong', '', `${t(kind)} · ${t('readOnly')}`));
  const close = element('button', 'btn ghost', '×');
  close.setAttribute('aria-label', t('close'));
  close.onclick = () => dialog.close();
  header.append(close);
  const rows = formatRows(kind, data).map(({key, value}) => {
    const row = element('div', 'format-row');
    const label = t(key);
    const display = formatValue(kind, key, value);
    row.append(element('span', '', label), element('span', 'mono', display));
    return row;
  });
  dialog.replaceChildren(header, ...rows);
  dialog.setAttribute('aria-label', t(kind));
  const position = () => {
    const bounds = dialog.getBoundingClientRect();
    const viewport = {width: window.innerWidth, height: window.innerHeight};
    const anchor = slip?.isConnected ? slip.getBoundingClientRect() : null;
    const placement = formatCardPosition(bounds, viewport, anchor);
    dialog.style.left = `${placement.left}px`;
    dialog.style.top = `${placement.top}px`;
  };
  dialog.onclose = () => {
    window.removeEventListener('resize', position);
    focus?.focus({preventScroll: true});
  };
  dialog.showModal();
  position();
  window.addEventListener('resize', position);
}
