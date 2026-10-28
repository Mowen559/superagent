#!/usr/bin/env node
/**
 * 海阔视界 (HikerView) 规则短口令与云剪贴板导出工具
 * 
 * 用法:
 * node export-command.cjs <规则文件.json> [provider: pasteme|cmdim]
 */

const fs = require('fs');
const path = require('path');

function sanitizeRule(rule) {
  const clean = { ...rule };
  delete clean.id;
  delete clean._id;
  delete clean.create_time;
  delete clean.update_time;
  clean.version = Number.isInteger(clean.version) && clean.version > 0 ? clean.version : 1;
  return clean;
}

function exportToStandardCommand(rule) {
  const clean = sanitizeRule(rule);
  const title = String(clean.title || '海阔规则').trim();
  const jsonStr = JSON.stringify(clean);
  const b64 = Buffer.from(jsonStr, 'utf8').toString('base64');
  return `海阔视界规则分享，当前分享的是：小程序【${title}】￥home_rule_v2￥base64://@${title}@${b64}`;
}

async function exportToCloudPaste(rule, options = {}) {
  const clean = sanitizeRule(rule);
  const title = String(clean.title || '海阔规则').trim();
  const standardCommand = exportToStandardCommand(clean);
  const provider = String(options.provider || 'pasteme').toLowerCase();

  // 1. 尝试 PasteMe (云6oooole)
  if (provider === 'pasteme' || provider === 'auto') {
    try {
      const res = await fetch('https://pasteme.tyrantg.com/api/create', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'
        },
        body: JSON.stringify({ content: standardCommand }),
        signal: AbortSignal.timeout(10000)
      });
      if (res.ok) {
        const json = await res.json();
        if (json && json.result_code === 'SUCCESS' && json.data?.path) {
          const codeId = json.data.path;
          const cloudCommand = `云6oooole/xxxxxx/${codeId}`;
          const fullShareCommand = `${cloudCommand}\n小程序：${title}`;
          const webUrl = `https://pasteme.tyrantg.com/${codeId}`;
          return {
            success: true,
            provider: 'pasteme',
            cloudCommand,
            fullShareCommand,
            codeId,
            webUrl,
            standardCommand,
            title
          };
        }
      }
    } catch (err) {
      console.warn('[ExportCommand] PasteMe 上传超时或失败，尝试备用 cmd.im:', err.message);
    }
  }

  // 2. 备用 cmd.im (云5oooole)
  try {
    const res = await fetch('https://cmd.im/', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/x-www-form-urlencoded',
        'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'
      },
      body: `txt=${encodeURIComponent(standardCommand)}`,
      redirect: 'manual',
      signal: AbortSignal.timeout(10000)
    });
    const location = res.headers.get('location');
    if (location) {
      const codeId = location.replace(/^\/+/, '');
      const cloudCommand = `云5oooole/xxxxxx/${codeId}`;
      const fullShareCommand = `${cloudCommand}\n小程序：${title}`;
      const webUrl = `https://cmd.im/${codeId}`;
      return {
        success: true,
        provider: 'cmdim',
        cloudCommand,
        fullShareCommand,
        codeId,
        webUrl,
        standardCommand,
        title
      };
    }
  } catch (err) {
    console.warn('[ExportCommand] cmd.im 上传失败:', err.message);
  }

  // 3. 上传云端均失败时降级返回离线标准口令
  return {
    success: false,
    error: '云剪贴板上传遇到网络波动，已生成标准离线口令',
    cloudCommand: '',
    fullShareCommand: standardCommand,
    standardCommand,
    title
  };
}

if (require.main === module) {
  const filePath = process.argv[2];
  const provider = process.argv[3] || 'pasteme';
  if (!filePath) {
    console.log('用法: node export-command.cjs <规则文件.json> [provider: pasteme|cmdim]');
    process.exit(1);
  }
  const absPath = path.resolve(process.cwd(), filePath);
  if (!fs.existsSync(absPath)) {
    console.error(`文件不存在: ${absPath}`);
    process.exit(1);
  }
  const rule = JSON.parse(fs.readFileSync(absPath, 'utf8'));
  exportToCloudPaste(rule, { provider }).then(async res => {
    console.log('\n================ 海阔规则口令导出结果 ================');
    if (res.cloudCommand) {
      console.log('【在线云短口令 (复制发给手机即可导入)】:');
      console.log(res.fullShareCommand);
      console.log('\n【云端直达网页】:', res.webUrl);
    }
    console.log('\n【离线标准口令 (Base64)】:');
    console.log(res.standardCommand.slice(0, 120) + '... (总长: ' + res.standardCommand.length + ')');

    // 探测 PC 本地 Python 爬虫引擎与局域网 52020 海阔视界设备
    try {
      const ping = await fetch('http://127.0.0.1:18000/api/v1/datasources', { signal: AbortSignal.timeout(600) });
      if (ping.ok) {
        console.log('\n【PC Python 爬虫引擎】: 🟢 本地服务在线 (127.0.0.1:18000)');
        console.log('  支持通过 POST /api/v1/hiker/push 将该规则一键静默推送到局域网海阔 App！');
      }
    } catch (_) {}

    try {
      const pingHiker = await fetch('http://127.0.0.1:52020/getAllRuleTitles', { signal: AbortSignal.timeout(600) });
      if (pingHiker.ok) {
        console.log('【局域网 52020 设备】: 🟢 探测到海阔视界处于在线监听状态，可免口令直推！');
      }
    } catch (_) {}

    console.log('======================================================\n');
  }).catch(err => {
    console.error('导出失败:', err);
    process.exit(1);
  });
}

module.exports = { exportToCloudPaste, exportToStandardCommand };
