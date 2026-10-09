// Only package files are loaded from disk. Documents arrive as framed bytes.
import { readFileSync } from 'node:fs';

const MAX_JSON = 4096;
const MAX_DOCUMENT = 64 * 1024 * 1024;
const MAX_PARAGRAPHS = 100000;

function requireRequest(condition, message) {
    if (!condition) {
        const error = new Error(message);
        error.code = 'BAD_REQUEST';
        throw error;
    }
}

async function readRequest() {
    const chunks = [];
    let size = 0;
    for await (const chunk of process.stdin) {
        size += chunk.length;
        requireRequest(size <= MAX_DOCUMENT + MAX_JSON + 4, 'Request exceeds the input limit.');
        chunks.push(chunk);
    }
    const input = Buffer.concat(chunks);
    requireRequest(input.length >= 4, 'Missing JSON length header.');
    const length = input.readUInt32LE(0);
    requireRequest(length > 0 && length <= MAX_JSON && input.length >= 4 + length, 'Invalid JSON frame length.');
    const request = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(input.subarray(4, 4 + length)));
    requireRequest(request !== null && typeof request === 'object' && !Array.isArray(request), 'Expected a request object.');
    requireRequest(Object.keys(request).every(key => ['op', 'page', 'format'].includes(key)), 'Unknown request field.');
    requireRequest(['probe', 'render', 'export'].includes(request.op), 'Unknown operation.');
    requireRequest(request.op === 'render' || request.page === undefined, 'Page is only valid for render.');
    requireRequest(request.op === 'export' || request.format === undefined, 'Format is only valid for export.');
    if (request.op === 'render') {
        requireRequest(Number.isInteger(request.page) && request.page >= 0, 'Page must be a nonnegative integer.');
    }
    if (request.op === 'export') {
        requireRequest(['hwp', 'hwpx'].includes(request.format), 'Export format must be hwp or hwpx.');
    }
    const bytes = input.subarray(4 + length);
    requireRequest(bytes.length > 0 && bytes.length <= MAX_DOCUMENT, 'Document must be between 1 byte and 64 MiB.');
    return { request, bytes };
}

function probe(document, version) {
    const sections = [];
    let count = 0;
    let truncated = false;
    for (let section = 0; section < document.getSectionCount(); section += 1) {
        const paragraphs = [];
        const paragraphCount = document.getParagraphCount(section);
        for (let paragraph = 0; paragraph < paragraphCount; paragraph += 1) {
            if (count === MAX_PARAGRAPHS) {
                truncated = true;
                break;
            }
            const length = document.getParagraphLength(section, paragraph);
            const text = document.getTextRange(section, paragraph, 0, length);
            paragraphs.push({ index: paragraph, text });
            count += 1;
        }
        sections.push({ index: section, paragraphs });
        if (truncated) {
            break;
        }
    }
    const result = { engine: 'rhwp', version, pageCount: document.pageCount(), sections };
    if (truncated) {
        result.truncated = true;
    }
    return result;
}

async function main() {
    const { request, bytes } = await readRequest();
    const packageUrl = new URL('../../node_modules/@rhwp/core/', import.meta.url);
    const { version } = JSON.parse(readFileSync(new URL('package.json', packageUrl), 'utf8'));
    const { default: init, HwpDocument } = await import(new URL('rhwp.js', packageUrl).href);
    // Node has no canvas. These approximate metrics enable layout queries, but
    // do not establish installed-font or Hancom layout equivalence.
    globalThis.measureTextWidth = (font, text) => Array.from(text).length * 10;
    await init({ module_or_path: readFileSync(new URL('rhwp_bg.wasm', packageUrl)) });
    const document = new HwpDocument(new Uint8Array(bytes));
    try {
        if (request.op === 'probe') {
            return probe(document, version);
        }
        if (request.op === 'render') {
            requireRequest(request.page < document.pageCount(), 'Page is outside the document.');
            return { page: request.page, svg: document.renderPageSvg(request.page) };
        }
        const exported = request.format === 'hwp' ? document.exportHwpWithReport() : document.exportHwpxWithReport();
        try {
            const lossReport = JSON.parse(exported.contentLoss());
            if (!exported.hasBytes()) {
                const error = new Error('rhwp did not produce export bytes.');
                error.code = 'EXPORT_FAILED';
                throw error;
            }
            return { format: request.format, lossReport, bytesBase64: Buffer.from(exported.takeBytes()).toString('base64') };
        } finally {
            exported.free();
        }
    } finally {
        document.free();
    }
}

try {
    process.stdout.write(JSON.stringify(await main()) + '\n');
} catch (error) {
    const code = error.code === 'BAD_REQUEST' || error instanceof SyntaxError || error instanceof TypeError
        ? 'BAD_REQUEST' : error.code || 'RHWP_FAILED';
    process.stdout.write(JSON.stringify({ error: { code, message: String(error.message || error) } }) + '\n');
    process.exitCode = 2;
}
