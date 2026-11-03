/* Read-only installed-binary metadata scan. No GUI, DLL execution or account data. */
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const [appDir, output] = process.argv.slice(2);
assert.ok(appDir && output, 'usage: node inspect_dmghg_protocol.cjs <app-dir> <new-output>');
assert.ok(!fs.existsSync(output), 'never overwrite protocol evidence');
const files = ['electron_bridge.dll', 'data/app.so'];
const facts = [];
for (const name of files) {
  const file = path.join(appDir, name);
  const bytes = fs.readFileSync(file);
  const text = bytes.toString('latin1');
  const fragments = [...text.matchAll(/[\x20-\x7e]{5,}/g)].map(m => ({ offset: m.index, value: m[0] }));
  const urls = new Set();
  for (const m of text.matchAll(/https?:\/\/[a-zA-Z0-9.:_-]+(?:\/[a-zA-Z0-9_./-]*)?/g)) {
    try { const u = new URL(m[0]); urls.add(u.origin + u.pathname); } catch {}
  }
  const markers = fragments.filter(m => /(?:dmghg|cenmo|gateway|catalog\.|video\.play|encrypt|decrypt|signature|hmac|aes-|nonce)/i.test(m.value));
  facts.push({ name, bytes: bytes.length, sha256: crypto.createHash('sha256').update(bytes).digest('hex'),
    candidate_urls: [...urls], markers });
  assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), facts.at(-1).sha256);
}
fs.mkdirSync(path.dirname(path.resolve(output)), { recursive: true });
fs.writeFileSync(output, JSON.stringify({ diagnostic_only: true, pure_js_verified: false, target: path.resolve(appDir), files: facts }, null, 2), { flag: 'wx' });
// Restrict stdout to URLs without queries or user info and marker counts.
console.log(JSON.stringify(facts.map(f => ({ name: f.name, sha256: f.sha256,
  candidate_urls: f.candidate_urls.filter(u => /cenmo|dmghg|gateway/i.test(u)), markers: f.markers.length })), null, 2));
