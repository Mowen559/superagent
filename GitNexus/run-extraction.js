const { execSync } = require('child_process');

[13, 14, 15, 16].forEach((batchIndex) => {
  console.log(`Extracting batch ${batchIndex}...`);
  try {
    execSync(
      `node "D:\\cloud\\deepcloud\\super agent\\Understand-Anything\\understand-anything-plugin\\skills\\understand\\extract-structure.mjs" ".understand-anything/tmp/ua-file-analyzer-input-${batchIndex}.json" ".understand-anything/tmp/ua-file-extract-results-${batchIndex}.json"`,
      {
        cwd: 'D:/cloud/deepcloud/super agent/GitNexus',
        stdio: 'inherit',
      },
    );
  } catch (e) {
    console.error(`Error on batch ${batchIndex}:`, e);
  }
});
console.log('All extracted.');
