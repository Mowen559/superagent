const fs = require('fs');
const path = require('path');

const PROJECT_ROOT = 'D:/cloud/deepcloud/super agent/GitNexus';
const TMP_DIR = path.join(PROJECT_ROOT, '.understand-anything/tmp');
const OUT_DIR = path.join(PROJECT_ROOT, '.understand-anything/intermediate');
const BATCHES_FILE = path.join(OUT_DIR, 'batches.json');

const dataBatches = JSON.parse(fs.readFileSync(BATCHES_FILE, 'utf8'));

function getNeighborMap(batchIndex) {
  const batch = dataBatches.batches.find((b) => b.batchIndex === batchIndex);
  return batch ? batch.neighborMap : {};
}

function getBatchImportData(batchIndex) {
  const batch = dataBatches.batches.find((b) => b.batchIndex === batchIndex);
  return batch ? batch.batchImportData : {};
}

function generateSummary(file, category) {
  const name = path.basename(file.path);
  if (category === 'code' || category === 'script' || category === 'markup') {
    if (name.includes('test') || name.includes('spec'))
      return `Test suite for ${name.replace(/\.(test|spec)\..+$/, '')} functionality.`;
    if (name === 'index.ts' || name === 'index.js')
      return `Entry point and barrel file exporting module contents for ${path.dirname(file.path)}.`;
    return `Implements core functionality for ${name}, providing required classes and functions.`;
  }
  if (category === 'config') return `Configuration file controlling settings for ${name}.`;
  if (category === 'docs')
    return `Documentation providing context, usage, or guidelines in ${name}.`;
  if (category === 'infra')
    return `Infrastructure definition for deploying or building the project via ${name}.`;
  if (category === 'data') return `Data schema or definition file for ${name}.`;
  return `Project file for ${name}.`;
}

function getComplexity(lines) {
  if (lines < 50) return 'simple';
  if (lines <= 200) return 'moderate';
  return 'complex';
}

function getTags(file, category) {
  const tags = new Set();
  const name = path.basename(file.path).toLowerCase();

  if (
    name.includes('.test.') ||
    name.includes('.spec.') ||
    name.endsWith('_test.go') ||
    name.endsWith('test.py')
  )
    tags.add('test');
  if (name === 'index.ts' || name === 'index.js' || name === '__init__.py') {
    tags.add('entry-point');
    tags.add('barrel');
  }
  if (category === 'config') tags.add('configuration');
  if (category === 'docs') tags.add('documentation');
  if (category === 'infra') tags.add('infrastructure');
  if (category === 'data') tags.add('schema-definition');
  if (category === 'code') tags.add('component');

  if (tags.size === 0) tags.add('utility');

  return Array.from(tags).slice(0, 5);
}

function getInfrastructureType(filePath) {
  const lower = filePath.toLowerCase();
  if (lower.includes('dockerfile') || lower.includes('docker-compose') || lower.includes('k8s'))
    return 'service';
  if (
    lower.includes('.github/workflows') ||
    lower.includes('.gitlab-ci') ||
    lower.includes('jenkinsfile')
  )
    return 'pipeline';
  if (lower.endsWith('.tf') || lower.includes('cloudformation')) return 'resource';
  return 'service';
}

function getDataType(filePath) {
  const lower = filePath.toLowerCase();
  if (lower.endsWith('.sql')) return 'table';
  if (lower.endsWith('.graphql') || lower.endsWith('.proto') || lower.endsWith('.prisma'))
    return 'schema';
  if (lower.endsWith('openapi.yaml') || lower.endsWith('swagger.json')) return 'endpoint';
  return 'table';
}

function determineNodeType(file, category) {
  if (category === 'config') return 'config';
  if (category === 'docs') return 'document';
  if (category === 'infra') return getInfrastructureType(file.path);
  if (category === 'data') return getDataType(file.path);
  return 'file';
}

function processBatch(batchIndex) {
  const resultsFile = path.join(TMP_DIR, `ua-file-extract-results-${batchIndex}.json`);
  if (!fs.existsSync(resultsFile)) return;

  const extraction = JSON.parse(fs.readFileSync(resultsFile, 'utf8'));
  const batchImportData = getBatchImportData(batchIndex);
  const neighborMap = getNeighborMap(batchIndex);

  const nodes = [];
  const edges = [];

  extraction.results.forEach((file) => {
    const type = determineNodeType(file, file.fileCategory);
    const prefix = type === 'file' ? 'file' : type;
    const fileId = `${prefix}:${file.path}`;

    // File node
    nodes.push({
      id: fileId,
      type: type,
      name: path.basename(file.path),
      filePath: file.path,
      summary: generateSummary(file, file.fileCategory),
      tags: getTags(file, file.fileCategory),
      complexity: getComplexity(file.nonEmptyLines || file.totalLines),
    });

    // Functions and classes
    const exportedNames = new Set((file.exports || []).map((e) => e.name));

    if (file.functions) {
      file.functions.forEach((fn) => {
        const lines = fn.endLine - fn.startLine;
        const isExported = exportedNames.has(fn.name);
        if (lines >= 10 || isExported) {
          const fnId = `function:${file.path}:${fn.name}`;
          nodes.push({
            id: fnId,
            type: 'function',
            name: fn.name,
            filePath: file.path,
            lineRange: [fn.startLine, fn.endLine],
            summary: `Function ${fn.name} implementation.`,
            tags: ['utility'],
            complexity: getComplexity(lines),
          });

          edges.push({
            source: fileId,
            target: fnId,
            type: 'contains',
            direction: 'forward',
            weight: 1.0,
          });

          if (isExported) {
            edges.push({
              source: fileId,
              target: fnId,
              type: 'exports',
              direction: 'forward',
              weight: 0.8,
            });
          }
        }
      });
    }

    if (file.classes) {
      file.classes.forEach((cls) => {
        const lines = cls.endLine - cls.startLine;
        const methodsCount = (cls.methods || []).length;
        const isExported = exportedNames.has(cls.name);
        if (lines >= 20 || methodsCount >= 2 || isExported) {
          const clsId = `class:${file.path}:${cls.name}`;
          nodes.push({
            id: clsId,
            type: 'class',
            name: cls.name,
            filePath: file.path,
            lineRange: [cls.startLine, cls.endLine],
            summary: `Class ${cls.name} implementation.`,
            tags: ['component'],
            complexity: getComplexity(lines),
          });

          edges.push({
            source: fileId,
            target: clsId,
            type: 'contains',
            direction: 'forward',
            weight: 1.0,
          });

          if (isExported) {
            edges.push({
              source: fileId,
              target: clsId,
              type: 'exports',
              direction: 'forward',
              weight: 0.8,
            });
          }
        }
      });
    }

    // Import edges
    const imports = batchImportData[file.path] || [];
    imports.forEach((imp) => {
      edges.push({
        source: fileId,
        target: `file:${imp}`,
        type: 'imports',
        direction: 'forward',
        weight: 0.7,
      });
    });
  });

  // Splitting logic
  const nodeCount = nodes.length;
  const edgeCount = edges.length;

  if (nodeCount <= 60 && edgeCount <= 120) {
    const outPath = path.join(OUT_DIR, `batch-${batchIndex}.json`);
    fs.writeFileSync(outPath, JSON.stringify({ nodes, edges }, null, 2));
    console.log(`Wrote ${outPath}`);
  } else {
    const parts = Math.ceil(Math.max(nodeCount / 60, edgeCount / 120));

    // Sort files
    const filePaths = Array.from(new Set(nodes.map((n) => n.filePath))).sort();
    const filesPerPart = Math.ceil(filePaths.length / parts);

    for (let i = 0; i < parts; i++) {
      const start = i * filesPerPart;
      const end = start + filesPerPart;
      const partFiles = new Set(filePaths.slice(start, end));

      const partNodes = nodes.filter((n) => partFiles.has(n.filePath));
      const partNodeIds = new Set(partNodes.map((n) => n.id));
      const partEdges = edges.filter((e) => partNodeIds.has(e.source));

      const outPath = path.join(OUT_DIR, `batch-${batchIndex}-part-${i + 1}.json`);
      fs.writeFileSync(outPath, JSON.stringify({ nodes: partNodes, edges: partEdges }, null, 2));
      console.log(`Wrote ${outPath}`);
    }
  }
}

[25, 26, 27, 28].forEach(processBatch);
