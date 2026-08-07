const fs = require('fs');
const data = JSON.parse(fs.readFileSync('.understand-anything/intermediate/batches.json', 'utf8'));

[13, 14, 15, 16].forEach((batchIndex) => {
  const b = data.batches.find((b) => b.batchIndex === batchIndex);
  if (!b) return;

  const input = {
    projectRoot: 'D:/cloud/deepcloud/super agent/GitNexus',
    batchFiles: b.files,
    batchImportData: b.batchImportData,
  };

  fs.mkdirSync('.understand-anything/tmp', { recursive: true });
  fs.writeFileSync(
    `.understand-anything/tmp/ua-file-analyzer-input-${batchIndex}.json`,
    JSON.stringify(input, null, 2),
  );

  fs.writeFileSync(
    `.understand-anything/tmp/batch-${batchIndex}-context.json`,
    JSON.stringify(
      {
        neighborMap: b.neighborMap,
        files: b.files,
        batchImportData: b.batchImportData,
      },
      null,
      2,
    ),
  );
});
console.log('Prepared inputs for batches 13-16');
