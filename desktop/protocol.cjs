const fs = require('node:fs/promises');
const {constants} = require('node:fs');
const path = require('node:path');

const MIME_TYPES = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.svg', 'image/svg+xml'],
  ['.png', 'image/png'],
  ['.wasm', 'application/wasm']
]);
const CSP = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'";

function inside(root, file) {
  const relative = path.relative(root, file);
  return relative !== '..' && !relative.startsWith('..' + path.sep) && !path.isAbsolute(relative);
}

async function serveAppRequest(request, roots) {
  try {
    if (request.method !== 'GET') {
      return new Response(null, {status: 405});
    }
    // Check before URL parsing, which otherwise normalizes literal dot segments.
    const decoded = decodeURIComponent(request.url);
    if (decoded.includes('\\') || decoded.includes('\0') || decoded.split(/[/?#]/).includes('..')) {
      return new Response(null, {status: 403});
    }
    const url = new URL(request.url);
    if (url.protocol !== 'app:' || url.host !== 'workbench' || url.username || url.password) {
      return new Response(null, {status: 403});
    }
    const pathname = decodeURIComponent(url.pathname);
    const isVendor = pathname.startsWith('/vendor/rhwp/');
    const root = await fs.realpath(isVendor ? roots.vendor : roots.desktop);
    const relative = isVendor ? pathname.slice('/vendor/rhwp/'.length) : pathname.slice(1);
    const file = path.resolve(root, relative || 'index.html');
    const mime = MIME_TYPES.get(path.extname(file));
    if (!mime || !inside(root, file) || (!isVendor && pathname.startsWith('/vendor/'))) {
      return new Response(null, {status: 403});
    }
    const resolved = await fs.realpath(file);
    if (!inside(root, resolved)) {
      return new Response(null, {status: 403});
    }
    const handle = await fs.open(resolved, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const stat = await handle.stat();
      if (!stat.isFile()) {
        return new Response(null, {status: 403});
      }
      const data = await handle.readFile();
      return new Response(data, {headers: {
        'content-type': mime,
        'content-security-policy': CSP,
        'x-content-type-options': 'nosniff'
      }});
    } finally {
      await handle.close();
    }
  } catch {
    return new Response(null, {status: 404});
  }
}

module.exports = {serveAppRequest};
