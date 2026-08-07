const fs = require('fs');
const path = require('path');

function processBatch(batchIndex) {
    const inputPath = `D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp/ua-file-analyzer-input-${batchIndex}.json`;
    const extractPath = `D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/tmp/ua-file-extract-results-${batchIndex}.json`;
    const outDir = `D:/cloud/deepcloud/super agent/GitNexus/.understand-anything/intermediate`;
    
    if (!fs.existsSync(extractPath)) {
        console.log(`Extract results for batch ${batchIndex} not found.`);
        return;
    }

    const inputData = JSON.parse(fs.readFileSync(inputPath, 'utf8'));
    const extractData = JSON.parse(fs.readFileSync(extractPath, 'utf8'));
    
    const nodes = [];
    const edges = [];
    
    for (const result of extractData.results) {
        const filePath = result.path;
        const cat = result.fileCategory;
        let type = "file";
        let summary = "";
        let tags = [];
        
        if (cat === 'code') {
            type = 'file';
            summary = `Code file implementing logic for ${path.basename(filePath)}.`;
            tags = ['code'];
            if (filePath.includes('.test.') || filePath.includes('.spec.') || filePath.includes('test_')) tags.push('test');
            if (filePath.endsWith('index.ts') || filePath.endsWith('index.js')) tags.push('barrel', 'entry-point');
        } else if (cat === 'config') {
            type = 'config';
            summary = `Configuration file for ${path.basename(filePath)}.`;
            tags = ['configuration'];
        } else if (cat === 'docs') {
            type = 'document';
            summary = `Documentation detailing ${path.basename(filePath)}.`;
            tags = ['documentation'];
        } else if (cat === 'infra') {
            if (filePath.includes('Dockerfile') || filePath.includes('docker-compose')) {
                type = 'service';
                tags = ['infrastructure', 'containerization'];
            } else if (filePath.includes('.github/workflows')) {
                type = 'pipeline';
                tags = ['ci-cd', 'deployment'];
            } else {
                type = 'resource';
                tags = ['infrastructure'];
            }
            summary = `Infrastructure definition for ${path.basename(filePath)}.`;
        } else if (cat === 'data') {
            if (filePath.endsWith('.sql')) {
                type = 'table';
                tags = ['database', 'migration'];
            } else if (filePath.endsWith('.graphql') || filePath.endsWith('.proto')) {
                type = 'schema';
                tags = ['schema-definition'];
            } else {
                type = 'endpoint';
                tags = ['api-schema'];
            }
            summary = `Data/schema definition for ${path.basename(filePath)}.`;
        } else if (cat === 'script') {
            type = 'file';
            summary = `Script file for automation or execution in ${path.basename(filePath)}.`;
            tags = ['script'];
        } else {
            type = 'file';
            summary = `Markup file for ${path.basename(filePath)}.`;
            tags = ['markup'];
        }

        const lines = result.nonEmptyLines || 0;
        let complexity = 'simple';
        if (lines >= 50 && lines <= 200) complexity = 'moderate';
        else if (lines > 200) complexity = 'complex';

        const fileNodeId = `${type}:${filePath}`;
        nodes.push({
            id: fileNodeId,
            type: type,
            name: path.basename(filePath),
            filePath: filePath,
            summary: summary,
            tags: [...new Set([...tags, 'source'])].slice(0, 5),
            complexity: complexity
        });

        // Functions
        const createdFunctions = new Set();
        if (result.functions) {
            for (const func of result.functions) {
                const lineCount = func.endLine - func.startLine + 1;
                const isExported = result.exports && result.exports.some(e => e.name === func.name);
                if (lineCount >= 10 || isExported) {
                    const funcId = `function:${filePath}:${func.name}`;
                    nodes.push({
                        id: funcId,
                        type: 'function',
                        name: func.name,
                        filePath: filePath,
                        lineRange: [func.startLine, func.endLine],
                        summary: `Function ${func.name} providing specific logic.`,
                        tags: ['function', 'logic', 'code'],
                        complexity: lineCount > 50 ? 'complex' : (lineCount > 20 ? 'moderate' : 'simple')
                    });
                    createdFunctions.add(func.name);
                    edges.push({
                        source: fileNodeId,
                        target: funcId,
                        type: 'contains',
                        direction: 'forward',
                        weight: 1.0
                    });
                    if (isExported) {
                        edges.push({
                            source: fileNodeId,
                            target: funcId,
                            type: 'exports',
                            direction: 'forward',
                            weight: 0.8
                        });
                    }
                }
            }
        }

        // Classes
        const createdClasses = new Set();
        if (result.classes) {
            for (const cls of result.classes) {
                const lineCount = cls.endLine - cls.startLine + 1;
                const isExported = result.exports && result.exports.some(e => e.name === cls.name);
                if (lineCount >= 20 || (cls.methods && cls.methods.length >= 2) || isExported) {
                    const clsId = `class:${filePath}:${cls.name}`;
                    nodes.push({
                        id: clsId,
                        type: 'class',
                        name: cls.name,
                        filePath: filePath,
                        lineRange: [cls.startLine, cls.endLine],
                        summary: `Class ${cls.name} encapsulating state and behavior.`,
                        tags: ['class', 'oop', 'code'],
                        complexity: lineCount > 100 ? 'complex' : (lineCount > 40 ? 'moderate' : 'simple')
                    });
                    createdClasses.add(cls.name);
                    edges.push({
                        source: fileNodeId,
                        target: clsId,
                        type: 'contains',
                        direction: 'forward',
                        weight: 1.0
                    });
                    if (isExported) {
                        edges.push({
                            source: fileNodeId,
                            target: clsId,
                            type: 'exports',
                            direction: 'forward',
                            weight: 0.8
                        });
                    }
                }
            }
        }

        // Imports
        if (inputData.batchImportData && inputData.batchImportData[filePath]) {
            for (const imp of inputData.batchImportData[filePath]) {
                edges.push({
                    source: fileNodeId,
                    target: `file:${imp}`,
                    type: 'imports',
                    direction: 'forward',
                    weight: 0.7
                });
            }
        }
    }

    // Split logic
    const nodeCount = nodes.length;
    const edgeCount = edges.length;
    
    if (nodeCount <= 60 && edgeCount <= 120) {
        fs.writeFileSync(path.join(outDir, `batch-${batchIndex}.json`), JSON.stringify({ nodes, edges }, null, 2));
        console.log(`Batch ${batchIndex}: Written single part.`);
    } else {
        const parts = Math.ceil(Math.max(nodeCount / 60, edgeCount / 120));
        
        // chunk files sequentially
        const sortedFiles = extractData.results.map(r => r.path).sort();
        const filesPerPart = Math.ceil(sortedFiles.length / parts);
        
        for (let i = 0; i < parts; i++) {
            const partFiles = new Set(sortedFiles.slice(i * filesPerPart, (i + 1) * filesPerPart));
            
            const partNodes = nodes.filter(n => partFiles.has(n.filePath));
            const partNodeIds = new Set(partNodes.map(n => n.id));
            const partEdges = edges.filter(e => {
                // edge belongs to this part if its SOURCE is in this part's nodes
                return partNodeIds.has(e.source);
            });
            
            fs.writeFileSync(path.join(outDir, `batch-${batchIndex}-part-${i + 1}.json`), JSON.stringify({ nodes: partNodes, edges: partEdges }, null, 2));
            console.log(`Batch ${batchIndex}: Written part ${i + 1} of ${parts}.`);
        }
    }
}

[21, 22, 23, 24].forEach(processBatch);
