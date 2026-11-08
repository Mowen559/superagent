const fs = require('node:fs');

// Windows readers/indexers can temporarily deny replacement of a report.
// Keep the old checkpoint intact; retry only transient sharing errors.
function writeJsonCheckpoint(file, value, { rename = fs.renameSync,
  wait = ms => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms) } = {}) {
  const temporary = file + '.tmp';
  fs.writeFileSync(temporary, JSON.stringify(value, null, 2));
  for (let attempt = 0; ; attempt++) {
    try { rename(temporary, file); return; }
    catch (error) {
      if (!['EPERM', 'EACCES', 'EBUSY'].includes(error.code) || attempt >= 9) throw error;
      wait(Math.min(20 * (2 ** attempt), 100));
    }
  }
}

module.exports = { writeJsonCheckpoint };
