#!/usr/bin/env node
/**
 * 海阔视界 (HikerView) 移动端与 PC 端双端同构规则全自动静态审计工具
 *
 * 用法:
 * node validate-rule.js <规则文件路径.json>
 * 或通过代码引入: const { validateRule } = require('./validate-rule.js');
 */

const fs = require('fs');
const path = require('path');
const vm = require('vm');

function validateRule(ruleContent) {
  const issues = [];
  const warnings = [];
  const passes = [];

  let rule = null;
  if (typeof ruleContent === 'string') {
    try {
      rule = JSON.parse(ruleContent);
    } catch (e) {
      issues.push(`【致命错误】JSON 语法解析失败: ${e.message}`);
      return { valid: false, issues, warnings, passes };
    }
  } else if (typeof ruleContent === 'object' && ruleContent !== null) {
    rule = ruleContent;
  } else {
    issues.push('【致命错误】输入内容非合法对象或 JSON 字符串');
    return { valid: false, issues, warnings, passes };
  }

  // 1. 基础元数据校验
  if (!rule.title || typeof rule.title !== 'string') {
    issues.push('缺少必填字段: title (规则名称)');
  } else if (rule.title.includes('__') || rule.title.trim() !== rule.title) {
    warnings.push(`规则名称建议去除多余下划线或空白字符: "${rule.title}"`);
  } else {
    passes.push(`规则名称合法: "${rule.title}"`);
  }

  if (typeof rule.version !== 'number' || !Number.isInteger(rule.version) || rule.version <= 0) {
    issues.push(`版本号 version 必须是正整数 (当前: ${JSON.stringify(rule.version)})，严禁使用 "1.0" 等字符串`);
  } else {
    passes.push(`版本号合法: ${rule.version}`);
  }

  if (!rule.url || typeof rule.url !== 'string') {
    issues.push('缺少必填字段: url (一级分类入口地址)');
  } else {
    // 移动端违禁域名黑名单拦截 (移动端弹窗: 频道链接包含违禁域名，无法保存，请支持正版)
    const forbiddenDomains = [
      'v.qq.com', 'qq.com', 'iqiyi.com', 'youku.com', 'bilibili.com', 
      'mgtv.com', 'le.com', 'sohu.com', 'xigua.com', 'xiaohongshu.com', 'douyin.com', 'kuaishou.com'
    ];
    const hitDomain = forbiddenDomains.find(d => rule.url.toLowerCase().includes(d));
    if (hitDomain) {
      issues.push(`【移动端保存阻断】频道链接 url 包含敏感正版域名 "${hitDomain}"！在移动端海阔视界点击保存时会强制弹出「频道链接包含违禁域名，无法保存，请支持正版」并拒绝保存！规范解法：必须使用海阔标准的 "hiker://empty#fyclass#fypage" 虚拟通道作为入口 url，将真实目标地址下沉至 find_rule 内部通过变量动态拼接。`);
    } else {
      passes.push('频道链接 url 未命中移动端违禁域名黑名单');
    }
  }

  if (rule.search_url && typeof rule.search_url === 'string') {
    const forbiddenDomains = ['v.qq.com', 'qq.com', 'iqiyi.com', 'youku.com', 'bilibili.com', 'mgtv.com'];
    const hitSearch = forbiddenDomains.find(d => rule.search_url.toLowerCase().includes(d));
    if (hitSearch) {
      issues.push(`【移动端保存阻断】搜索链接 search_url 包含敏感正版域名 "${hitSearch}"！建议使用 "hiker://empty?q=**" 虚拟协议并在 search_rule 内部动态请求。`);
    } else {
      passes.push('搜索链接 search_url 合规');
    }
  }
  const jsFields = ['find_rule', 'search_rule', 'detail_find_rule', 'lazy'];
  const dummyContext = {
    input: 'https://example.com',
    request: () => '<html></html>',
    pdfh: () => '',
    pdfa: () => [],
    pd: () => '',
    setResult: () => {},
    console: { log: () => {} },
  };

  for (const field of jsFields) {
    const code = rule[field];
    if (!code) continue;

    // 2.1 裸浏览器对象检测 (移动端直接崩溃)
    if (/\b(?:window|document|DOMParser|XMLHttpRequest|navigator\.userAgent)\b/.test(code)) {
      issues.push(`字段 [${field}] 违规调用了裸浏览器对象 (window/document/DOMParser)，移动端无 DOM 环境会直接抛出 ReferenceError 崩溃！`);
    } else {
      passes.push(`字段 [${field}] 未检测到裸浏览器私有对象`);
    }

    // 2.1.1 移动端 Rhino 现代语法不兼容拦截 (?. 与 ?? 致命 SyntaxError 阻断)
    if (/(?:\?\.|\?\?)/.test(code)) {
      issues.push(`字段 [${field}] 检测到使用了现代 ES2020+ 可选链 (?.) 或空值合并 (??) 语法！移动端海阔视界大部分依赖 Mozilla Rhino 引擎，对 ?. 与 ?? 语法会直接抛出 SyntaxError: Unexpected token 并崩溃导致整页白屏！必须转换为 ES5/ES6 经典语法: (a && a.b) 或三元表达式 (a !== undefined ? a : b)！`);
    } else {
      passes.push(`字段 [${field}] 未使用移动端不兼容的 ?. 或 ?? 现代语法`);
    }

    // 2.2 语法有效性静态检测
    if (typeof code === 'string' && code.startsWith('js:')) {
      const cleanCode = code.slice(3);
      try {
        new vm.Script(`(function(){\n${cleanCode}\n})`);
        passes.push(`字段 [${field}] JS 脚本语法校验通过`);
      } catch (err) {
        issues.push(`字段 [${field}] JS 语法错误: ${err.message}`);
      }
    }

    // 2.3 移动端 MyJsoup 逗号多选择器陷阱检测 (java.lang.NumberFormatException: For input string: ".xxx")
    const commaSelectorMatch = code.match(/pdf[ah]\s*\([^,)]+,\s*["'][^"']*,[^"']*["']\s*\)/g);
    if (commaSelectorMatch) {
      issues.push(`字段 [${field}] 违规在选择器中使用逗号 [${commaSelectorMatch[0]}]！移动端 MyJsoup 会将逗号后内容误当作整型切片索引 (Integer.parseInt)，直接触发 JavaException NumberFormatException 崩溃！请改用空格链式或单选择器条件降级。`);
    } else {
      passes.push(`字段 [${field}] 未使用移动端违禁逗号复合选择器`);
    }

    // 2.4 二级页面裸用 input 变量检测 (移动端 ReferenceError: "input" 未定义)
    if (field === 'detail_find_rule') {
      if (/\binput\.(?:match|replace|split|indexOf|slice)\b/.test(code) && !code.includes('MY_URL')) {
        issues.push(`字段 [${field}] 裸调用了 "input" 变量！查阅移动端 Android 原生 JSEngine.java，二级页面仅注入了 MY_URL 与 MY_PAGE，裸写 input 会在手机端抛出 ReferenceError 崩溃！必须在首行使用同构垫片: var currentUrl = (typeof MY_URL !== 'undefined' && MY_URL) ? MY_URL : (typeof input !== 'undefined' ? input : '');`);
      } else {
        passes.push(`字段 [${field}] 双端变量垫片校验合规`);
      }
    }
  }

  // 3. 选集 lazy 协议检测 (@lazyRule= 强制标准)
  if (rule.detail_find_rule) {
    const detailCode = String(rule.detail_find_rule);
    if (detailCode.includes('url: \'lazy:\'') || detailCode.includes('url: "lazy:"') || detailCode.includes('\'lazy:\' +') || detailCode.includes('"lazy:" +')) {
      issues.push('选集条目检测到违禁的伪前缀 "lazy:"！查阅海阔 Android 源码 ArticleListFragment.java，引擎只切分 "@lazyRule="，写 "lazy:" 会被误判为普通页面递归调用 detail_find_rule 导致死循环与播放失败！必须改用标准协议: url: epUrl + "@lazyRule=.js:(() => { return input; })()"');
    } else if (detailCode.includes('@lazyRule=')) {
      passes.push('选集协议已规范化采用标准 "@lazyRule=" 惰性求值');
    } else if (detailCode.includes('col_type: \'pic_1_full\'')) {
      passes.push('图集漫画规则，直接输出大图瀑布流');
    } else if (detailCode.includes('@rule=')) {
      passes.push('采用多层级 @rule= 级联路由模式');
    } else {
      warnings.push('选集条目建议统一加上 "@lazyRule=" 前缀，实现点击时按需即时解析，杜绝全量解析导致的 IP 封禁与 Token 过期');
    }

    // 4. 多层级 @rule= 级联路由深度审计 (Multi-Tier Navigation)
    if (detailCode.includes('@rule=')) {
      passes.push('检测到多层级 @rule= 级联路由架构 (支持季/线路卡片多层下潜)');
      if (!detailCode.includes('seriesTitle') && !detailCode.includes('parentTitle') && !detailCode.includes('剧名')) {
        warnings.push('多层级下潜规则中，下游页面首项建议持续传递并注入 "剧名：XXX"，防止三级页面历史记录退化为纯线路名');
      }
    }

    // 5. 剧名上下文保护检测 (契约台账 #280)
    if (!detailCode.includes('剧名') && !detailCode.includes('pic_1_card') && !detailCode.includes('seriesTitle')) {
      warnings.push('详情页顶部建议显式注入 "剧名：XXX" 大卡片，确保 PC 桌面端播放历史和首页卡片能够精准识别大剧名，避免误存为纯分集名（如 "第01集"）');
    } else {
      passes.push('二级详情页包含剧名上下文保护卡片');
    }
  }

  // 5. 防盗链注解检测
  const fullRuleStr = JSON.stringify(rule);
  if (!fullRuleStr.includes('@Referer=') && !fullRuleStr.includes(';{Referer@')) {
    warnings.push('规则中未检测到防盗链注解 (@Referer= 或 ;{Referer@...})，若目标源有防盗链限制，可能在 PC 桌面端导致图片碎裂或播放 403 阻断');
  } else {
    passes.push('包含标准防盗链注解 (@Referer= / ;{Referer@...})');
  }

  return {
    valid: issues.length === 0,
    issues,
    warnings,
    passes,
  };
}

// 命令行执行支持
if (require.main === module) {
  const targetFile = process.argv[2];
  if (!targetFile) {
    console.log('使用说明: node validate-rule.js <规则文件.json>');
    process.exit(1);
  }
  const absPath = path.resolve(process.cwd(), targetFile);
  if (!fs.existsSync(absPath)) {
    console.error(`文件不存在: ${absPath}`);
    process.exit(1);
  }
  const content = fs.readFileSync(absPath, 'utf8');
  const result = validateRule(content);

  console.log('\n================ 海阔规则双端同构静态审计报告 ================');
  console.log(`审计结果: ${result.valid ? '✅ 通过 (合规)' : '❌ 失败 (存在阻断性问题)'}`);
  
  if (result.passes.length > 0) {
    console.log('\n【合规项】:');
    result.passes.forEach(p => console.log('  ✓ ' + p));
  }
  if (result.warnings.length > 0) {
    console.log('\n【优化建议】:');
    result.warnings.forEach(w => console.log('  ⚠️ ' + w));
  }
  if (result.issues.length > 0) {
    console.log('\n【阻断缺陷】:');
    result.issues.forEach(i => console.log('  ✗ ' + i));
  }
  console.log('===============================================================\n');

  process.exit(result.valid ? 0 : 1);
}

module.exports = { validateRule };
