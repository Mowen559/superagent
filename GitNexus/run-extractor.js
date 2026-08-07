const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const projectRoot = 'D:/cloud/deepcloud/super agent/GitNexus';
const batchesPath = path.join(projectRoot, '.understand-anything/intermediate/batches.json');
const tmpDir = path.join(projectRoot, '.understand-anything/tmp');
const extractScript =
  'D:/cloud/deepcloud/super agent/Understand-Anything/understand-anything-plugin/skills/understand/extract-structure.mjs';

if (!fs.existsSync(tmpDir)) {
  fs.mkdirSync(tmpDir, { recursive: true });
}

const data = JSON.parse(fs.readFileSync(batchesPath, 'utf8'));
const targetBatches = [1, 2, 3, 4];

for (const batchIndex of targetBatches) {
  const batch = data.batches.find((b) => b.batchIndex === batchIndex);
  if (!batch) {
    console.error(`Batch ${batchIndex} not found`);
    continue;
  }

  const inputJson = {
    projectRoot: projectRoot,
    batchFiles: batch.files,
    batchImportData: batch.batchImportData,
  };

  const inputPath = path.join(tmpDir, `ua-file-analyzer-input-${batchIndex}.json`);
  const outputPath = path.join(tmpDir, `ua-file-extract-results-${batchIndex}.json`);

  fs.writeFileSync(inputPath, JSON.stringify(inputJson, null, 2));

  console.log(`Running extraction for batch ${batchIndex}...`);
  try {
    execSync(`node "${extractScript}" "${inputPath}" "${outputPath}"`, { stdio: 'inherit' });
    console.log(`Successfully extracted batch ${batchIndex}`);
  } catch (error) {
    console.error(`Failed to extract batch ${batchIndex}:`, error.message);
  }
}
