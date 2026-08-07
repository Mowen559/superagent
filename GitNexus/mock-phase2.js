const fs = require('fs');
const path = require('path');

const root = 'D:\\cloud\\deepcloud\\super agent\\GitNexus';
const batchesPath = path.join(root, '.understand-anything', 'intermediate', 'batches.json');
const outDir = path.join(root, '.understand-anything', 'intermediate');

const data = JSON.parse(fs.readFileSync(batchesPath, 'utf8'));

for (let i = 0; i < data.batches.length; i++) {
  const batch = data.batches[i];
  const nodes = [];
  const edges = [];

  for (const f of batch.files) {
    const fPath = f.path.replace(/\\/g, '/');
    const name = fPath.split('/').pop();
    const type =
      f.fileCategory === 'code'
        ? 'file'
        : f.fileCategory === 'config'
          ? 'config'
          : f.fileCategory === 'docs'
            ? 'document'
            : 'file';

    const nodeId = `${type}:${fPath}`;
    nodes.push({
      id: nodeId,
      type: type,
      name: name,
      filePath: fPath,
      summary: `Automated summary for ${name}`,
      tags: [type, 'mock-generated'],
    });
  }

  const outData = { nodes, edges };
  fs.writeFileSync(path.join(outDir, `batch-${i}.json`), JSON.stringify(outData, null, 2));
}

console.log(`Generated ${data.batches.length} batch JSONs.`);
