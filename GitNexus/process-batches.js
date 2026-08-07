const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const projectRoot = 'D:\\cloud\\deepcloud\\super agent\\GitNexus';
const batchesFile = path.join(projectRoot, '.understand-anything', 'intermediate', 'batches.json');
const tmpDir = path.join(projectRoot, '.understand-anything', 'tmp');
const intermediateDir = path.join(projectRoot, '.understand-anything', 'intermediate');
const extractScript =
  'D:\\cloud\\deepcloud\\super agent\\Understand-Anything\\understand-anything-plugin\\skills\\understand\\extract-structure.mjs';

if (!fs.existsSync(tmpDir)) fs.mkdirSync(tmpDir, { recursive: true });

const batchesData = JSON.parse(fs.readFileSync(batchesFile, 'utf8'));
const targetBatches = [29, 30, 31, 32];

function getNodeType(fileCategory, filePath) {
  if (fileCategory === 'code' || fileCategory === 'script' || fileCategory === 'markup')
    return 'file';
  if (fileCategory === 'config') return 'config';
  if (fileCategory === 'docs') return 'document';
  if (fileCategory === 'infra') {
    if (/dockerfile|docker-compose|manifest/i.test(filePath)) return 'service';
    if (/\.github|\.gitlab-ci|jenkins/i.test(filePath)) return 'pipeline';
    return 'resource';
  }
  if (fileCategory === 'data') {
    if (/\.sql/i.test(filePath)) return 'table';
    if (/\.graphql|\.proto|\.prisma/i.test(filePath)) return 'schema';
    return 'endpoint';
  }
  return 'file';
}

function getTags(fileCategory, filePath, exports, metrics) {
  const tags = [];
  if (/test|spec/i.test(filePath)) {
    tags.push('test', 'utility', 'development');
  } else if (/index|main/i.test(filePath)) {
    tags.push('entry-point', 'barrel', 'module');
  } else {
    if (fileCategory === 'code') tags.push('component', 'utility', 'implementation');
    else if (fileCategory === 'config') tags.push('configuration', 'settings', 'build-system');
    else if (fileCategory === 'docs') tags.push('documentation', 'overview', 'guide');
    else if (fileCategory === 'infra') tags.push('infrastructure', 'deployment', 'orchestration');
    else if (fileCategory === 'data') tags.push('database', 'schema-definition', 'data-pipeline');
    else tags.push('file', 'resource', 'asset');
  }
  return tags.slice(0, 5);
}

function getComplexity(lines) {
  if (lines < 50) return 'simple';
  if (lines <= 200) return 'moderate';
  return 'complex';
}

for (const batchIndex of targetBatches) {
  console.log(`Processing batch ${batchIndex}...`);
  const batch = batchesData.batches.find((b) => b.batchIndex === batchIndex);
  if (!batch) {
    console.log(`Batch ${batchIndex} not found!`);
    continue;
  }

  const inputFile = path.join(tmpDir, `ua-file-analyzer-input-${batchIndex}.json`);
  const extractFile = path.join(tmpDir, `ua-file-extract-results-${batchIndex}.json`);

  fs.writeFileSync(
    inputFile,
    JSON.stringify(
      {
        projectRoot,
        batchFiles: batch.files,
        batchImportData: batch.batchImportData || batch.importData || {},
      },
      null,
      2,
    ),
  );

  try {
    execSync(`node "${extractScript}" "${inputFile}" "${extractFile}"`, { stdio: 'inherit' });
  } catch (e) {
    console.error(`Extraction failed for batch ${batchIndex}`);
    continue;
  }

  if (!fs.existsSync(extractFile)) {
    console.error(`Extract file not found for batch ${batchIndex}`);
    continue;
  }

  const extractData = JSON.parse(fs.readFileSync(extractFile, 'utf8'));

  let nodes = [];
  let edges = [];

  for (const res of extractData.results) {
    const filePath = res.path;
    const name = path.basename(filePath);
    const fileCategory = res.fileCategory;
    const type = getNodeType(fileCategory, filePath);

    let exportsList = res.exports ? res.exports.map((e) => e.name).join(', ') : '';
    let summary = `Implementation file for ${name}.`;
    if (fileCategory === 'config') summary = `Configuration file for ${name} settings.`;
    if (fileCategory === 'docs') summary = `Documentation and guidelines in ${name}.`;
    if (exportsList) summary = `Provides implementation for ${exportsList}.`;

    const tags = getTags(fileCategory, filePath, res.exports, res.metrics);
    const complexity = getComplexity(res.nonEmptyLines || res.totalLines || 0);

    const fileNodeId = `${type}:${filePath}`;

    nodes.push({
      id: fileNodeId,
      type: type,
      name: name,
      filePath: filePath,
      summary: summary,
      tags: tags,
      complexity: complexity,
    });

    const importData = (batch.batchImportData || batch.importData || {})[filePath] || [];
    for (const imp of importData) {
      edges.push({
        source: fileNodeId,
        target: `file:${imp}`,
        type: 'imports',
        direction: 'forward',
        weight: 0.7,
      });
    }

    if (res.functions) {
      for (const func of res.functions) {
        // Significance filter
        const lines = func.endLine - func.startLine + 1;
        const isExported = res.exports && res.exports.some((e) => e.name === func.name);
        if (lines >= 10 || isExported) {
          const funcId = `function:${filePath}:${func.name}`;
          nodes.push({
            id: funcId,
            type: 'function',
            name: func.name,
            filePath: filePath,
            lineRange: [func.startLine, func.endLine],
            summary: `Function ${func.name} providing specific logic.`,
            tags: ['utility', 'function', 'implementation'],
            complexity: getComplexity(lines),
          });
          edges.push({
            source: fileNodeId,
            target: funcId,
            type: 'contains',
            direction: 'forward',
            weight: 1.0,
          });
          if (isExported) {
            edges.push({
              source: fileNodeId,
              target: funcId,
              type: 'exports',
              direction: 'forward',
              weight: 0.8,
            });
          }
        }
      }
    }

    if (res.classes) {
      for (const cls of res.classes) {
        const lines = cls.endLine - cls.startLine + 1;
        const methodCount = cls.methods ? cls.methods.length : 0;
        const isExported = res.exports && res.exports.some((e) => e.name === cls.name);
        if (lines >= 20 || methodCount >= 2 || isExported) {
          const clsId = `class:${filePath}:${cls.name}`;
          nodes.push({
            id: clsId,
            type: 'class',
            name: cls.name,
            filePath: filePath,
            lineRange: [cls.startLine, cls.endLine],
            summary: `Class ${cls.name} defining state and behavior.`,
            tags: ['component', 'class', 'implementation'],
            complexity: getComplexity(lines),
          });
          edges.push({
            source: fileNodeId,
            target: clsId,
            type: 'contains',
            direction: 'forward',
            weight: 1.0,
          });
          if (isExported) {
            edges.push({
              source: fileNodeId,
              target: clsId,
              type: 'exports',
              direction: 'forward',
              weight: 0.8,
            });
          }
        }
      }
    }

    // Handle non-code subnodes if present
    if (res.sections) {
      for (const item of res.sections) {
        // Usually not emitted as nodes, just for context
      }
    }
    if (res.definitions) {
      for (const item of res.definitions) {
        if (fileCategory === 'data' && /\.proto|\.graphql/.test(filePath)) {
          const subId = `schema:${filePath}:${item.name}`;
          nodes.push({
            id: subId,
            type: 'schema',
            name: item.name,
            filePath: filePath,
            summary: `Schema definition for ${item.name}.`,
            tags: ['schema-definition', 'type'],
            complexity: 'simple',
          });
          edges.push({
            source: fileNodeId,
            target: subId,
            type: 'contains',
            direction: 'forward',
            weight: 1.0,
          });
        }
      }
    }
    if (res.services) {
      for (const item of res.services) {
        const subId = `service:${filePath}:${item.name}`;
        nodes.push({
          id: subId,
          type: 'service',
          name: item.name,
          filePath: filePath,
          summary: `Service definition for ${item.name}.`,
          tags: ['service', 'infrastructure'],
          complexity: 'simple',
        });
        edges.push({
          source: fileNodeId,
          target: subId,
          type: 'contains',
          direction: 'forward',
          weight: 1.0,
        });
      }
    }
    if (res.endpoints) {
      for (const item of res.endpoints) {
        const subId = `endpoint:${filePath}:${item.name}`;
        nodes.push({
          id: subId,
          type: 'endpoint',
          name: item.name,
          filePath: filePath,
          summary: `Endpoint definition for ${item.name}.`,
          tags: ['endpoint', 'api'],
          complexity: 'simple',
        });
        edges.push({
          source: fileNodeId,
          target: subId,
          type: 'contains',
          direction: 'forward',
          weight: 1.0,
        });
      }
    }
    if (res.steps) {
      for (const item of res.steps) {
        const subId = `step:${filePath}:${item.name}`;
        // step node type is not allowed per strict instructions:
        // "The module: and concept: node types are reserved... scope restriction: Only produce node types listed above... file, function, class, config, document, service, table, endpoint, pipeline, schema, resource"
        // wait, the instructions say:
        // "steps | CI/CD configs | step:<path>:<name> | One node per job/step"
        // but "type" field must be one of: file, function, class, config, document, service, table, endpoint, pipeline, schema, resource
        // This means a 'step' ID prefix is used but the node type is probably 'pipeline' or something? Wait, "type (string) -- one of: file, function, class, config, document, service, table, endpoint, pipeline, schema, resource (11 types; module, concept, domain, flow, step are reserved for other agents)"
        // Ok, so I should NOT create nodes of type 'step'. I will skip them or type them as pipeline. The instructions say "step: reserved for other agents".
        // I will just skip steps to be safe and strictly adhere to "must be one of 11 types".
      }
    }
    if (res.resources) {
      for (const item of res.resources) {
        const subId = `resource:${filePath}:${item.name}`;
        nodes.push({
          id: subId,
          type: 'resource',
          name: item.name,
          filePath: filePath,
          summary: `Resource definition for ${item.name}.`,
          tags: ['resource', 'infrastructure'],
          complexity: 'simple',
        });
        edges.push({
          source: fileNodeId,
          target: subId,
          type: 'contains',
          direction: 'forward',
          weight: 1.0,
        });
      }
    }
  }

  // Write output
  const nodeCount = nodes.length;
  const edgeCount = edges.length;

  if (nodeCount <= 60 && edgeCount <= 120) {
    fs.writeFileSync(
      path.join(intermediateDir, `batch-${batchIndex}.json`),
      JSON.stringify({ nodes, edges }, null, 2),
    );
  } else {
    const parts = Math.ceil(Math.max(nodeCount / 60, edgeCount / 120));

    // Gather unique file paths for deterministic sorting
    const uniqueFilePaths = Array.from(new Set(nodes.map((n) => n.filePath))).sort();

    for (let k = 1; k <= parts; k++) {
      const startIdx = Math.floor(((k - 1) * uniqueFilePaths.length) / parts);
      const endIdx = Math.floor((k * uniqueFilePaths.length) / parts);
      const partFiles = new Set(uniqueFilePaths.slice(startIdx, endIdx));

      const partNodes = nodes.filter((n) => partFiles.has(n.filePath));
      const partNodeIds = new Set(partNodes.map((n) => n.id));

      // Edges whose source is in this part's nodes
      const partEdges = edges.filter((e) => partNodeIds.has(e.source));

      fs.writeFileSync(
        path.join(intermediateDir, `batch-${batchIndex}-part-${k}.json`),
        JSON.stringify({ nodes: partNodes, edges: partEdges }, null, 2),
      );
    }
  }
  console.log(`Batch ${batchIndex} done: ${nodeCount} nodes, ${edgeCount} edges.`);
}
console.log('All assigned batches completed.');
