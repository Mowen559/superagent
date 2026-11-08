const test = require('node:test'), assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const { writeJsonCheckpoint } = require('./hub-workflow-checkpoint.cjs');
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hub-checkpoint-'));
  t.after(() => {
    const base = fs.realpathSync(os.tmpdir()), target = fs.realpathSync(root);
    assert.ok(target.toLowerCase().startsWith((base + path.sep).toLowerCase()));
    fs.rmSync(target, { recursive: true, force: true });
  });
  const file = path.join(root, 'report.json'); fs.writeFileSync(file, '{"old":true}'); return file;
}
test('transient sharing errors retry without exposing a partial report', t => {
  const file = fixture(t); let attempts = 0; const waits = [];
  writeJsonCheckpoint(file, { updated: true }, { rename: (source, target) => {
    attempts++; if (attempts < 3) { assert.deepEqual(JSON.parse(fs.readFileSync(target)), { old: true }); throw Object.assign(Error('sharing'), { code: 'EPERM' }); }
    fs.renameSync(source, target);
  }, wait: ms => waits.push(ms) });
  assert.equal(attempts, 3); assert.deepEqual(waits, [20, 40]);
  assert.deepEqual(JSON.parse(fs.readFileSync(file)), { updated: true });
});
test('permanent sharing failure is bounded and preserves the prior report', t => {
  const file = fixture(t); let attempts = 0;
  assert.throws(() => writeJsonCheckpoint(file, { updated: true }, { rename: () => {
    attempts++; throw Object.assign(Error('still locked'), { code: 'EACCES' });
  }, wait: () => {} }));
  assert.equal(attempts, 10); assert.deepEqual(JSON.parse(fs.readFileSync(file)), { old: true });
});
test('unrelated IO errors fail immediately', t => {
  const file = fixture(t); let attempts = 0;
  assert.throws(() => writeJsonCheckpoint(file, {}, { rename: () => {
    attempts++; throw Object.assign(Error('IO failure'), { code: 'EIO' });
  }, wait: () => assert.fail('must not retry') }));
  assert.equal(attempts, 1); assert.deepEqual(JSON.parse(fs.readFileSync(file)), { old: true });
});
