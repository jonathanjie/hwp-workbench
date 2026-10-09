// Test-only direct getTextRange queries. No document path crosses the bridge.
import { readFileSync } from 'node:fs';
import init, { HwpDocument } from '../node_modules/@rhwp/core/rhwp.js';
const chunks = [];
for await (const chunk of process.stdin) {
    chunks.push(chunk);
}
const input = Buffer.concat(chunks);
const length = input.readUInt32LE(0);
const request = JSON.parse(input.subarray(4, 4 + length));
globalThis.measureTextWidth = (font, text) => Array.from(text).length * 10;
await init({ module_or_path: readFileSync(new URL('../node_modules/@rhwp/core/rhwp_bg.wasm', import.meta.url)) });
const document = new HwpDocument(new Uint8Array(input.subarray(4 + length)));
try {
    const ranges = {};
    for (const { id, anchor, count } of request.nodes) {
        ranges[id] = document.getTextRange(anchor.section, anchor.paragraph, anchor.charOffset, count);
    }
    process.stdout.write(JSON.stringify({ ranges }));
} finally {
    document.free();
}
