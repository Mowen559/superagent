#!/usr/bin/env node
/**
 * 海阔视界 (HikerView) 规则动态微调与补丁工具 (Rule Patcher)
 * 
 * 用法:
 * node patch-rule.cjs <输入规则.json> <输出规则.json> [选项: --title "新标题" --col_type "movie_2" --bump-version]
 */

const fs = require('fs');
const path = require('path');

function patchRule(rule, patches = {}) {
  const next = { ...rule };

  if (patches.title) next.title = String(patches.title).trim();
  if (patches.author) next.author = String(patches.author).trim();
  if (patches.col_type) next.col_type = String(patches.col_type).trim();
  if (patches.detail_col_type) next.detail_col_type = String(patches.detail_col_type).trim();
  if (patches.url) next.url = String(patches.url).trim();
  if (patches.search_url) next.search_url = String(patches.search_url).trim();
  if (patches.class_name) next.class_name = String(patches.class_name).trim();
  if (patches.class_url) next.class_url = String(patches.class_url).trim();
  
  if (patches.bumpVersion) {
    next.version = (typeof next.version === 'number' ? next.version : 1) + 1;
  } else if (typeof patches.version === 'number') {
    next.version = patches.version;
  }

  // 替换防盗链基准 Referer
  if (patches.referer) {
    const ref = patches.referer.replace(/\/+$/, '');
    const replaceRef = (code) => {
      if (typeof code !== 'string') return code;
      return code
        .replace(/@Referer=[^'"`&;\s]+/g, `@Referer=${ref}/`)
        .replace(/;{Referer@[^&;\s]+/g, `;{Referer@${ref}/`);
    };
    next.find_rule = replaceRef(next.find_rule);
    next.search_rule = replaceRef(next.search_rule);
    next.detail_find_rule = replaceRef(next.detail_find_rule);
    next.lazy = replaceRef(next.lazy);
  }

  return next;
}

if (require.main === module) {
  const args = process.argv.slice(2);
  if (args.length < 2) {
    console.log('用法: node patch-rule.cjs <输入.json> <输出.json> [--title "新标题"] [--col_type "movie_2"] [--bump-version] [--referer "https://..."]');
    process.exit(1);
  }

  const [inputPath, outputPath] = args;
  const patches = {};

  for (let i = 2; i < args.length; i++) {
    const a = args[i];
    if (a === '--bump-version') patches.bumpVersion = true;
    else if (a === '--title' && args[i + 1]) patches.title = args[++i];
    else if (a === '--col_type' && args[i + 1]) patches.col_type = args[++i];
    else if (a === '--referer' && args[i + 1]) patches.referer = args[++i];
    else if (a === '--url' && args[i + 1]) patches.url = args[++i];
  }

  const absIn = path.resolve(process.cwd(), inputPath);
  const rule = JSON.parse(fs.readFileSync(absIn, 'utf8'));
  const patched = patchRule(rule, patches);

  const absOut = path.resolve(process.cwd(), outputPath);
  fs.writeFileSync(absOut, JSON.stringify(patched, null, 2), 'utf8');
  console.log(`\n微调补丁已成功应用至: ${outputPath}`);
  console.log('更新后标题:', patched.title);
  console.log('更新后版本:', patched.version);
  console.log('更新后布局:', patched.col_type);
}

module.exports = { patchRule };
