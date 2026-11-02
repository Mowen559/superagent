// Parser diagnostic only: never writes or executes the business rule.
const fs = require('node:fs');
const vm = require('node:vm');
const crypto = require('node:crypto');
const input = fs.readFileSync(process.argv[2]);
const detail = JSON.parse(input).detail_find_rule;
const match = detail.match(/url:\s*playParamUrl\s*\+\s*('(?:[^'\\]|\\.)*')/);
if (!match) throw Error('The expected nested string literal was not found');
const expanded = vm.runInNewContext(match[1]);
const source = expanded.slice('@lazyRule='.length).replace(/^\.js:/, '');
const result = { diagnostic_only: true, source_sha256: crypto.createHash('sha256').update(input).digest('hex'),
  decoded_lazy_compiles: false, decoded_regex_became_comment: source.includes('replace(//$/'), source_modified: false };
try { new Function(`return (${source})`); result.decoded_lazy_compiles = true; }
catch (error) { result.parse_error = error.message; }
fs.writeFileSync(process.argv[3], JSON.stringify(result, null, 2), {flag:'wx'});
console.log(JSON.stringify(result));
