export function macFontFamily(family) {
  if (/sans|gothic|고딕|돋움|굴림/i.test(family)) {
    return 'Apple SD Gothic Neo';
  }
  if (/명조|바탕|batang|myeongjo|serif/i.test(family)) {
    return 'AppleMyungjo';
  }
  return 'Apple SD Gothic Neo';
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
    const family = list.split(',')[0].trim().replace(/^['"]|['"]$/g, '');
    if (available(family)) {
      return family;
    }
    const replacement = macFontFamily(family);
    if (family !== replacement && !substitutions.has(family)) {
      substitutions.set(family, replacement);
      onSubstitutions([...substitutions].map(([from, to]) => ({from, to})));
    }
    return replacement;
  }
  function measure(font, text) {
    const match = /^(.*?\d+(?:\.\d+)?px(?:\/[\d.]+)?\s+)(.+)$/.exec(font);
    context.font = match ? `${match[1]}"${mapFamily(match[2])}"` : font;
    return context.measureText(text).width;
  }
  function mapSvg(svg) {
    // Parse detached markup only to change font attributes. Pages enter the
    // application as image blobs, never as document-provided DOM elements.
    const parsed = new DOMParser().parseFromString(svg, 'image/svg+xml');
    for (const text of parsed.querySelectorAll('[font-family]')) {
      text.setAttribute('font-family', mapFamily(text.getAttribute('font-family')));
    }
    return new XMLSerializer().serializeToString(parsed);
  }
  return {measure, mapSvg};
}
