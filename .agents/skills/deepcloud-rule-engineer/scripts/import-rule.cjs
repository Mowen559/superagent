#!/usr/bin/env node
/**
 * 海阔视界 (HikerView) 多模态规则导入解析器
 * 
 * 支持输入:
 * 1. 本地 JSON 文件路径
 * 2. 在线云短口令 (如 云6oooole/xxxxxx/<id> 或 云5oooole/xxxxxx/<id>)
 * 3. 云剪贴板网址 (如 https://pasteme.tyrantg.com/<id>)
 * 4. 官方离线标准口令 (￥home_rule_v2￥base64://...)
 * 5. 纯 JSON 文本
 * 
 * 用法:
 * node import-rule.cjs "<口令/URL/文件路径>" [输出文件.json]
 */

const fs = require('fs');
const path = require('path');

async function importRule(inputStr) {
  if (!inputStr || typeof inputStr !== 'string') {
    throw new Error('无效的输入文本');
  }

  const trimmed = inputStr.trim();

  // 1. 本地文件路径判定
  if (fs.existsSync(trimmed)) {
    const raw = fs.readFileSync(trimmed, 'utf8');
    return importRule(raw);
  }

  // 2. 纯 JSON 检测
  if (trimmed.startsWith('{') && trimmed.endsWith('}')) {
    try {
      const parsed = JSON.parse(trimmed);
      return { success: true, rule: parsed, source: 'raw_json' };
    } catch (_) {}
  }

  // 3. 官方 Base64 标准口令 (￥home_rule_v2￥base64://@标题@<b64>)
  if (trimmed.includes('￥home_rule_v2￥base64://')) {
    const b64Part = trimmed.split('￥home_rule_v2￥base64://')[1] || '';
    // 剥离可能存在的 @标题@ 前缀
    const cleanB64 = b64Part.replace(/^@[^@]+@/, '').trim();
    try {
      const jsonStr = Buffer.from(cleanB64, 'base64').toString('utf8');
      const parsed = JSON.parse(jsonStr);
      return { success: true, rule: parsed, source: 'standard_command' };
    } catch (err) {
      throw new Error(`离线标准口令 Base64 解码失败: ${err.message}`);
    }
  }

  // 4. 云剪贴板短口令与网页链接
  // A. PasteMe (云6oooole / pasteme.tyrantg.com)
  if (trimmed.includes('云6oooole') || trimmed.includes('pasteme')) {
    const match = trimmed.match(/(?:pasteme\.tyrantg\.com\/|云6oooole\/(?:xxxxxx\/)?)([a-zA-Z0-9_\-\/@]+)/i);
    if (match && match[1]) {
      const codeId = match[1].split('@')[0];
      const apiUrl = `https://pasteme.tyrantg.com/api/getContent/${codeId}`;
      const res = await fetch(apiUrl, { signal: AbortSignal.timeout(10000) });
      if (!res.ok) throw new Error(`云剪贴板 PasteMe 请求失败: HTTP ${res.status}`);
      const json = await res.json();
      const rawContent = typeof json.data === 'string' ? json.data : (json.data?.content || '');
      if (json && json.result_code === 'SUCCESS' && rawContent) {
        return importRule(rawContent);
      }
      throw new Error(json?.message || '未获取到有效云剪贴板内容');
    }
  }

  // B. cmd.im (云5oooole / cmd.im)
  if (trimmed.includes('云5oooole') || trimmed.includes('cmd.im')) {
    const match = trimmed.match(/(?:cmd\.im\/|云5oooole\/(?:xxxxxx\/)?)([a-zA-Z0-9_\-]+)/i);
    if (match && match[1]) {
      const codeId = match[1];
      const apiUrl = `https://cmd.im/${codeId}?raw=1`;
      const res = await fetch(apiUrl, { signal: AbortSignal.timeout(10000) });
      if (!res.ok) throw new Error(`云剪贴板 cmd.im 请求失败: HTTP ${res.status}`);
      const rawText = await res.text();
      return importRule(rawText);
    }
  }

  throw new Error('未能识别该口令或规则格式，请检查内容是否有效');
}

if (require.main === module) {
  const inputArg = process.argv[2];
  const outPath = process.argv[3];
  if (!inputArg) {
    console.log('用法: node import-rule.cjs "<口令/URL/文件路径>" [输出文件.json]');
    process.exit(1);
  }

  importRule(inputArg).then(res => {
    console.log('\n================ 海阔规则导入解析成功 ================');
    console.log('来源模式:', res.source);
    console.log('规则标题:', res.rule.title);
    console.log('版本号:', res.rule.version);
    console.log('入口 URL:', res.rule.url);
    console.log('分类:', res.rule.class_name);
    
    if (outPath) {
      fs.writeFileSync(path.resolve(process.cwd(), outPath), JSON.stringify(res.rule, null, 2), 'utf8');
      console.log(`\n已成功保存至文件: ${outPath}`);
    }
    console.log('======================================================\n');
  }).catch(err => {
    console.error('导入解析失败:', err.message);
    process.exit(1);
  });
}

module.exports = { importRule };
