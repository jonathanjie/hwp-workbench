// Shared by canvas/SVG rendering and the requested-font Checks list.
export const FONT_SUBSTITUTIONS = Object.freeze({
  '맑은 고딕': 'Apple SD Gothic Neo', '굴림': 'Apple SD Gothic Neo', '돋움': 'Apple SD Gothic Neo',
  '함초롬돋움': 'Apple SD Gothic Neo', '바탕': 'AppleMyungjo', '궁서': 'AppleMyungjo',
  '함초롬바탕': 'AppleMyungjo', 'Malgun Gothic': 'Apple SD Gothic Neo',
  'Gulim': 'Apple SD Gothic Neo', 'Dotum': 'Apple SD Gothic Neo',
  'Batang': 'AppleMyungjo', 'Gungsuh': 'AppleMyungjo'
});

export function requestedFontSubstitutions(families, available) {
  return [...new Set(families)].filter(family => !available(family)).map(from => ({from, to: macFontFamily(from)}));
}

export function macFontFamily(family) {
  if (Object.hasOwn(FONT_SUBSTITUTIONS, family)) {
    return FONT_SUBSTITUTIONS[family];
  }
  if (/sans|gothic|고딕|돋움|굴림/i.test(family)) {
    return 'Apple SD Gothic Neo';
  }
  if (/명조|바탕|궁서|batang|gungsuh|myeongjo|serif/i.test(family)) {
    return 'AppleMyungjo';
  }
  return 'Apple SD Gothic Neo';
}

// Canvas measurements and SVG text must use the same complete family list.
export function substituteFontFamily(list, available, onSubstitution = () => {}) {
  const family = list.split(',')[0].trim().replace(/^['"]|['"]$/g, '');
  const replacement = available(family) ? family : macFontFamily(family);
  if (replacement !== family) {
    onSubstitution(family, replacement);
  }
  const fallback = /명조|바탕|궁서|batang|gungsuh|myeongjo|serif/i.test(replacement) || replacement === 'AppleMyungjo' ? 'serif' : 'sans-serif';
  return `"${replacement.replaceAll('"', '')}", ${fallback}`;
}

export function createFontMapper(onSubstitutions) {
  const context = document.createElement('canvas').getContext('2d');
  const installed = new Map();
  const substitutions = new Map();
  const sample = '가나다 ABC mw0123456789';
  function available(family) {
    if (!installed.has(family)) {
      // FontFaceSet.check reports true for absent system fonts. Compare metrics
      // against two generic families to detect actual system-font fallback.
      const widths = ['monospace', 'serif'].map(fallback => {
        context.font = `16px ${fallback}`;
        const baseline = context.measureText(sample).width;
        context.font = `16px "${family.replaceAll('"', '')}", ${fallback}`;
        return context.measureText(sample).width !== baseline;
      });
      installed.set(family, widths.some(Boolean));
    }
    return installed.get(family);
  }
  function mapFamily(list) {
    return substituteFontFamily(list, available, (family, replacement) => {
      if (!substitutions.has(family)) {
        substitutions.set(family, replacement);
        onSubstitutions([...substitutions].map(([from, to]) => ({from, to})));
      }
    });
  }
  function measure(font, text) {
    const match = /^(.*?\d+(?:\.\d+)?px(?:\/[\d.]+)?\s+)(.+)$/.exec(font);
    context.font = match ? `${match[1]}${mapFamily(match[2])}` : font;
    return context.measureText(text).width;
  }
  function mapSvg(svg) {
    // Parse detached markup only to change font attributes. Pages enter the
    // application as image blobs, never as document-provided DOM elements.
    const parsed = new DOMParser().parseFromString(svg, 'image/svg+xml');
    for (const text of parsed.querySelectorAll('[font-family]')) {
      text.setAttribute('font-family', mapFamily(text.getAttribute('font-family')));
    }
    for (const element of parsed.querySelectorAll('[style]')) {
      const family = element.style.getPropertyValue('font-family');
      if (family) {
        element.style.setProperty('font-family', mapFamily(family));
      }
    }
    return new XMLSerializer().serializeToString(parsed);
  }
  function requested(families) {
    for (const {from, to} of requestedFontSubstitutions(families, available)) {
      substitutions.set(from, to);
    }
    onSubstitutions([...substitutions].map(([from, to]) => ({from, to})));
  }
  return {measure, mapSvg, requested};
}
