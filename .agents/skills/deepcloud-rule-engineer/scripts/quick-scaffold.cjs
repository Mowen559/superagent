#!/usr/bin/env node
/**
 * 快速规则脚手架生成工具 (Quick Rule Scaffold)
 * 
 * 用法:
 * node quick-scaffold.js <规则名称> <入口URL> [类型: vod|json|comic|drama]
 * 示例:
 * node quick-scaffold.js "西瓜影院" "https://xigua.com" vod
 */

const fs = require('fs');
const path = require('path');

const typeMap = {
  vod: 'maccms-vod.template.json',
  json: 'json-api.template.json',
  comic: 'comic-manga.template.json',
  drama: 'short-drama.template.json',
  cascade: 'multi-tier-cascade.template.json',
  multi: 'multi-tier-cascade.template.json',
};

function scaffold(name, siteUrl, type = 'vod') {
  const templateName = typeMap[type] || 'maccms-vod.template.json';
  const templatePath = path.resolve(__dirname, '../templates', templateName);
  
  if (!fs.existsSync(templatePath)) {
    throw new Error(`找不到模板文件: ${templatePath}`);
  }

  const raw = fs.readFileSync(templatePath, 'utf8');
  let replaced = raw
    .replace(/站点名称/g, name)
    .replace(/https:\/\/example\.com/g, siteUrl.replace(/\/+$/, ''));

  return JSON.parse(replaced);
}

if (require.main === module) {
  const [,, name, url, type] = process.argv;
  if (!name || !url) {
    console.log('用法: node quick-scaffold.js <规则名称> <入口URL> [类型: vod|json|comic|drama]');
    process.exit(1);
  }
  const result = scaffold(name, url, type || 'vod');
  console.log(JSON.stringify(result, null, 2));
}

module.exports = { scaffold };
