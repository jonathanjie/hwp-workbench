import {t, warningLabel} from './i18n.js';
import {element} from './dom.js';

const WARNING_FEATURES = {
  LAYOUT_UNVERIFIED: 'layoutUnverified', FONT_METRICS_UNVERIFIED: 'fonts', TABLE_LAYOUT: 'tables',
  IMAGES: 'images', PREVIEW_CACHE_RETAINED: 'previewCache', HWP_EQUAL_LENGTH_ONLY: 'sameLengthFeature',
  HWP_LAYOUT_NOT_RENDERED: 'structure', PROTECTED_OR_ACTIVE_CONTENT: 'locked', DEFAULT_SCRIPT_STUB: 'scriptStub',
  EQUATIONS: 'equations', OLE_OBJECTS: 'ole', CHARTS: 'charts', HEADERS_FOOTERS: 'headers',
  FOOTNOTES: 'footnotes', FIELDS: 'fields', SHAPES: 'shapes', PAGE_NUMBERING: 'numbering', BOOKMARKS: 'bookmarks'
};

export function featureChecks(documentData) {
  const checks = [
    {key: 'textEdits', state: documentData.capabilities.textEdit ? 'ok' : 'off'},
    {key: 'formatting', state: documentData.capabilities.formattingEdit ? 'ok' : 'off'},
    {key: 'structureEdits', state: documentData.capabilities.structuralEdit ? 'ok' : 'off'}
  ];
  for (const warning of documentData.warnings) {
    const key = WARNING_FEATURES[warning.code];
    const state = warning.severity === 'error' || ['LAYOUT_UNVERIFIED', 'PROTECTED_OR_ACTIVE_CONTENT'].includes(warning.code) ? 'off' : 'part';
    checks.push({key, code: warning.code, state, warning});
  }
  return checks;
}

export function compatibilityTags(documentData) {
  return featureChecks(documentData).map(check => {
    const state = check.state === 'part' ? 'partial' : check.state;
    const tag = element('span', `tag ${check.state}`, `${check.key ? t(check.key) : check.code} · ${t(state)}`);
    tag.dataset.feature = check.code || check.key;
    if (check.warning) {
      tag.title = warningLabel(check.warning);
    }
    return tag;
  });
}

export function renderChecks(documentData, tags, warnings) {
  tags.replaceChildren(...compatibilityTags(documentData));
  warnings.replaceChildren(...documentData.warnings.map(warning => {
    const row = element('div', `warning ${warning.severity}`);
    row.append(element('span', 'code', warning.code), element('div', '', warningLabel(warning)));
    // Preserve complete engine messages, including warnings unknown to this UI.
    row.append(element('div', 'engine-message', warning.message));
    return row;
  }));
}
