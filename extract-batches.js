const fs = require('fs');
const batchesData = JSON.parse(fs.readFileSync('D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/intermediate/batches.json', 'utf8'));
const indices = [21, 22, 23, 24];
const targetBatches = batchesData.batches.filter(b => indices.includes(b.batchIndex));

targetBatches.forEach(batch => {
  const inputFile = `D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp/ua-file-analyzer-input-${batch.batchIndex}.json`;
  const inputData = {
    projectRoot: 'D:/cloud/deepcloud/super agent/GitNexus',
    batchFiles: batch.files,
    batchImportData: batch.batchImportData
  };
  fs.mkdirSync('D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp', { recursive: true });
  fs.writeFileSync(inputFile, JSON.stringify(inputData, null, 2));
  fs.writeFileSync(`D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp/neighborMap-${batch.batchIndex}.json`, JSON.stringify(batch.neighborMap, null, 2));
});
