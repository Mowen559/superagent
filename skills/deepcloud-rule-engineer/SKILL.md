---
name: "deepcloud-rule-engineer"
description: >
  海阔规则工程：少量目标→调查证据→用户选方案→实现→用户实测反馈→修订。
  PC真实执行用scripts/test-rule-live.cjs，PC导入指引用references/pc-rule-import.md；test-rule.cjs仅仿真。
  支持网站规则、应用逆向、HikerView、独立JS、云剪切板与短口令，按用户当前范围读取适用资源。
---

# 移动端与 PC 端双端同构海阔规则极速生成与全生命周期工程 Skill (Isomorphic Hiker Rule Architect)

> **最高纲领**：按用户指定的平台交付可验证规则，明确实际依赖与联网要求；PC与Android分别认证。
> Android 原生海阔与 DeepCloud PC 必须分别认证。未实测的平台标注未验证，不能由 PC 或仿真结果推导手机可用。
> 覆盖「**极简输入 -> 移动端API深挖 -> PC引擎协同 -> 静态审计 -> 沙箱深度评分 -> 口令/局域网直推交付**」全生命周期工程闭环。

---

## 已安装应用优先路由

用户给本地安装路径是要求分析原应用，不代表同意规则依赖该应用。默认由local-app-inspector协同还原实际远端接口、请求参数、签名/加解密与媒体解析，再整理为目标海阔可直接运行的JS规则。DLL探针只能作为调查与原目标对照，不能替代协议逆向。用户要求独立JS时，禁止将Python桥接、Node调用DLL、本地HTTP服务或EXE包装当作最终交付；独立可用性必须在无这些依赖的规则运行环境验证。只有用户明确选择有依赖的本地方案时，才走原生桥接契约。尚未还原协议时记录具体缺口，不能凭“已识别DLL”断言只能桥接。

## 本地业务验收与受控外发顺序

解析器先用移动诊断脚本的--list-parsers列出实际资源路径、指纹及Java状态，或用--parser-profile选择登记资源；默认公开对照解析器不代表APK/手机版本。资源缺失或指纹不符须报告，不递归猜测安装目录，不把JAR就绪当作手机已验证。

嵌套eval还须检查当前页绑定：scripts/test-rule-mobile-context.cjs接受规则、实际生成的lazy源码、明确绑定的入口字段及可信解析器JAR，只运行外层并捕获/编译eval参数，不执行捕获代码或媒体。移动端MY_RULE.find_rule可能是当前页面脚本，不能由PC完整规则包上下文推导；按实际APK调用链核对绑定。无捕获则未覆盖，不算通过。

移动端语法须单独检查：scripts/test-rule-mobile-syntax.cjs接受实际解析器JAR与工作区rule.json，可用--source补充从真实详情生成的lazy或嵌套eval源码。工具只编译不执行，记录解析器哈希与入口错误，不能认证Android实机、网络请求或播放。仓库中的Rhino 1.7.13仅为已知基线；手机版本未知时不得把基线错误当作该手机报错的已证唯一根因。收到具体点击报错，先调查实际点击源码及MY_RULE注入边界，禁止仅凭V8静态成功判移动端已修复。

PC规则实际自检使用scripts/test-rule-live.cjs，先真实导入再执行home/search/detail/action；可用--request传入工作区请求JSON。该入口没有站点实现或验收答案，返回data供自行检查，不认证媒体解码、GUI或Android。test-rule.cjs是仿真，评分及退出码不能替代真实运行。PC导入说明先阅读references/pc-rule-import.md，遵循已选平台，禁止套用手机菜单。


取得足够真实数据并完成必要的用户方案选择后，再编写最小完整规则，再用实际运行错误迭代，避免反复探测已确认的接口。静态校验或星级不证明分类、搜索、详情、播放可用。必须执行实际生成的点击/lazy；只有列表或详情文字，仍不算播放成功。先排查生产库兼容性，不通过改目标或假视频掩盖错误。

若Run决策状态列出了managed_actions/action_id，先调用verify_delivery验证来源规则，再调用perform_delivery_action({action_id})发布。局部验证返回completion_ready=false表示其他交付尚未齐，不代表本地通过收据失效。不得自行运行导出CLI或通过shell试探云写API；不得上传dummy/test正文。成功ID必须保留，回读失败仅复验原ID，不再创建副本；没有可用更新协议时如实报告，禁止猜测PUT/POST。没有授权和注册动作时，不因工具名称存在而推断允许外发。

免Python/本机桥接不等于脱离网络。说明必须按实测区分PC运行库、GUI和Android，未测平台不能写成已适配或双端保证，画质以实际元数据/码率为准。云口令生成后仍需生产解析回读，并确认对应的是最终已验规则。

## 〇、少信息请求的调查与用户协作流程

用户通常只给一个网址、安装路径或一句使用意愿。先由智能体读取目标并调查公开接口、页面和运行条件，不把反爬、选择器、签名实现等技术问题转嫁给用户。调查时给出真实访问或响应证据，区分已验证事实、推断和未解决项，不直接套用相同站点名的旧规则。

发现会影响用户使用方式的实质选择时，先简要说明证据、可做功能、依赖及限制，再通过ask_user给出普通用户能理解的选项与推荐，等待用户选择。可询问主要使用电脑还是手机、优先独立运行还是接受额外服务、先交付哪些功能；不得要求用户提供代码、接口或协议答案。已有明确偏好和授权不重复确认；仅存在一种可行路径时说明理由，实质影响使用方式仍需用户选择。

用户选择后由Hub亲自生成规则、自行运行适用检查，交付可导入文件、简短步骤、已验证功能和明确未测范围，再邀请用户实际测试。收到功能反馈后，先依据现象和当前产物自行调查、修复，再给修订版与复测点。只在确实无法自行取得必要信息时询问关键现象。不得用内部自测冒充用户测试，不得编造菜单、自动弹窗或Android实机通过。

发布短口令和设备推送仍依明确授权。用户测试不通过时保留原版及真实失败，不以更换目标、弱化断言或已有参考规则替代完整链。说明所复用资产和知识的来源。

流程：少量目标信息 → Hub调查及证据 → 方案与使用代价 → 用户选择 → Hub实现与自检 → 用户测试 → Hub据反馈修复及复测 → 完成交付。

---

## 一、加解密、验证码与可选运行路径


- 默认还原目标实际协议并交付独立JS：请求认证、签名、响应解密及播放解析各自验证。加解密不是验证码/站点防护的通用解决器。
- 独立JS可内嵌适用算法库；PC的CryptoUtil/Java兼容接口和Android实际能力分别核验。不能由支持AES或RSA推导任意应用协议已还原。
- 图形验证码按规则提供输入/提交；WebView验证由客户端页面处理，需要交互时由用户完成，再复用当前设备会话Cookie并重新请求。存在Cookie不证明服务器已放行。
- Python/无头浏览器仅在用户明确接受额外本地服务时作为可选方案；运行时免Python的任务不得暗中探测本地服务、调用原应用或加载远端动态实现。动态渲染、浏览器隐身选项不能保证通过Cloudflare、Akamai或任意滑块。
- 遇到实际挑战、登录或授权缺口，报告原始失败类别与用户可操作入口，必要时等待用户完成验证；不以修改目标来源或伪造成功输出代替实测。

当前路径与验证边界见 [加解密与验证处理](references/crypto-and-verification.md)。仅在用户选择本地引擎依赖时再读 [可选Python协同](references/python-engine-synergy.md)，其中接口示例须核验，不能视作已认证依赖或默认交付。

---

## 二、海阔视界双端同构函数字典与页面生命周期规范 (API & Page Lifecycle)

海阔视界是一套卡片数据流驱动架构。在编写规则前，必须透彻掌握各页面生命周期、注入变量与原生排版类型：

### 1. 五大核心页面模型与上下文变量注入

| 页面类型 | 对应规则字段 | 触发时机 | 运行时注入变量 (Context Variables) | 返回驱动契约 |
|---|---|---|---|---|
| **一级频道页** | `url` + `find_rule` | 点击小程序、切换分类 Tab、下拉刷新、触底翻页 | • `MY_URL`: 当前频道 URL (已替换 fyclass/fypage)<br>• `MY_PAGE`: 当前页码 (首屏为 1)<br>• `MY_PARAMS.class`: PC当前分类标识（MY_CATE需按平台核对）<br>• `MY_RULE`: 规则元数据<br>⚠️ *未注入 input 变量，禁止裸写* | 必须调用 `setResult(d)` 输出卡片数组 |
| **全局搜索页** | `search_url` + `searchFind` | 顶部搜索栏输入关键字提交 | • `MY_URL`: 已将 `**` 替换为编码关键词的 URL<br>• `MY_KEYWORD` / `KEYWORD`: PC原始搜索词文本<br>• `MY_PAGE`: 分页页码 | 必须调用 `setResult(d)` 输出卡片数组 |
| **二级详情页** | `detail_col_type` + `detail_find_rule` | 点击列表卡片，移动端启动 `DetailActivity` | • `MY_URL`: 点击卡片的 `url` 属性<br>• `MY_PAGE`: 默认为 1<br>⚠️ *移动端仅注入 MY_URL，必须使用同构垫片* | 必须调用 `setResult(d)`；<br>**首卡必须为 `pic_1_card` 注入剧名契约** |
| **三级级联页** | 卡片 `url` 包含 `@rule=` | 点击多季卡片、多线路卡片或分段集数卡片 | • `MY_URL`: `@rule=` 前面的目标 URL<br>• 执行 `@rule=` 后面的下游解析代码 | 必须调用 `setResult(d)`；<br>首项必须持续传递父剧名 |
| **选集惰性解析** | 卡片 `url` 包含 `@lazyRule=` | 用户点击具体某个选集播放按钮 | • `input`: 包含选集的待解析地址<br>• 执行 `@lazyRule=` 后面的自执行函数 | 必须 `return playUrl` 返回真实播放直链 |

### 2. 原生卡片排版布局字典 (`col_type` Guide)

| `col_type` 布局 | 视觉表现 | 经典应用场景 | 关键规范与属性 |
|---|---|---|---|
| **`movie_3`** | 三列竖版海报卡片 (3:4) | 影视、动漫、长视频首页列表 | `title`, `pic_url`, `url`, `desc` |
| **`movie_2`** | 双列竖版瀑布流卡片 | 小红书社区、微短剧、抖音快手横流 | `title`, `pic_url`, `url`, `desc` |
| **`pic_1_card`** | 详情页大头卡片 (剧名保护) | 影视/笔记二级详情页首项 | `title: '剧名：' + name`, `pic_url`, `url` |
| **`pic_1_full`** | 全屏满宽大图 (自适应高) | 图集原图连续浏览、漫画阅读 | `title`, `pic_url`, `url: pic_url` (点击放大) |
| **`pic_3`** | 三列正方形九宫格相册 | 朋友圈动态、商品多图 | `title`, `pic_url`, `url` |
| **`text_1`** | 单行通栏大标题 / 分割线 | 线路切换标题（如【蓝光专线】）、正文段落 | `title`, `col_type: 'text_1'` |
| **`text_2`** | 双列中型卡片按钮 | 多线路切换、分季选择（第1季、第2季） | `title`, `desc`, `url` |
| **`text_3`** | 三列方块按钮 | 电视剧/动漫标准选集 (第01集、第02集) | `title`, `url` (带 `@lazyRule=`) |
| **`text_4`** | 四列紧凑型小方块 | 80+ 集超长短剧密集排布 | `title`, `url` (带 `@lazyRule=`) |
| **`flex_button`** | 自适应宽度流式胶囊按钮 | 快捷分类标签、热搜词、追问快捷按钮 | `title`, `url` |

### 3. 内置同构函数 API 清单
- **网络**：`request(url, options)`、`post(url, options)`；
- **解析**：`pdfa(html, selector)`（严禁带逗号！）、`pdfh(html, selector)`、`pd(html, selector, baseUrl)`；
- **存储**：`setItem(k, v)`、`getItem(k, defaultVal)`、`clearItem(k)`；
- **驱动**：`setResult(items)`；
- **反馈**：`toast(msg)`、`showLoading(text)`、`hideLoading()`。

📖 详细手册详见：[`references/hiker-api-lifecycle.md`](./references/hiker-api-lifecycle.md)。

---

## 三、图文多图画册与视频免嗅播放沉浸式处理规范

针对“图片不仅只有一张、视频如何保证真正播放”等深度诉求，必须严格执行以下实战策略：

### 1. 图文全量多图展开与画册浏览策略 (Multi-Image Gallery)
- **多图提取规范**：
  在详情页中，绝不允许只输出封面图一张图片！必须逆向详情数据中的多图数组（如 `imageList`, `photos`）：
  ```javascript
  // 遍历所有高清配图，以 pic_1_full 输出原生连续瀑布流
  for (var p = 0; p < imageList.length; p++) {
      var picUrl = imageList[p];
      d.push({
          title: '图 ' + (p + 1) + ' / ' + imageList.length,
          pic_url: picUrl + '@Referer=' + base + '/',
          url: picUrl + '@Referer=' + base + '/', // 点击直接唤起海阔原生画廊缩放查看
          col_type: 'pic_1_full'
      });
  }
  ```
- **渐进式风控穿透**：
  若在纯移动端遭遇类似小红书 `xhs_sec_server` 302 风控拦截：
  1. 首屏封面去水印高清化，作为大图保底；
  2. 规则提供「🌐 在海阔内置无痕浏览器中滑动查看完整图集与评论」入口，条目 `url` 设为 `input://https://www.xiaohongshu.com/explore/...`，用户在海阔内直接唤起全功能无痕 WebView 沉浸滑动，作为未经接口认证的浏览器备用入口，不保证绕过限制。

### 2. 视频真实直链与免嗅探探针策略 (Video Stream & Sniffer)
- **直链提取优先**：
  若逆向数据中包含真实流媒体字段（`m3u8`、`h264/masterUrl`、`mp4`），使用标准 `@lazyRule=` 输出：
  ```javascript
  d.push({
      title: '▶️ 播放视频笔记',
      url: videoUrl + '@lazyRule=.js:(() => { return input + ";{Referer@' + base + '/&&User-Agent@PC}"; })()',
      col_type: 'text_1'
  });
  ```
- **免嗅探协议兜底**：
  若视频源有动态签名保护，使用海阔视界原生的网页嗅探协议：
  `url = playPageUrl + '#isVideo=true##noLoading#'`；
  该协议只是请求客户端尝试嗅探；是否取得可播放地址、请求头及是否需要交互验证，须在实际目标客户端检查。

---

## 四、双端同构九大防踩坑铁律 (Isomorphic Iron Laws)

以下约束按目标和已验证平台适用；本地原生接口优先遵循其精简PC契约，不能把未验证的移动端行为覆盖到PC：

1. **🔴 铁律 1 (RULE-NO-DOM)**：严禁调用裸 `window` / `document` / `fetch`，统一使用 `request`、`pdfa`、`pdfh`、`pd`；
2. **🔴 铁律 2 (RULE-FORBIDDEN-DOMAIN-SHIELD)**：入口URL按实际平台契约选择；PC本地桥接使用许可端口的真实地址及统一分页，不追加普通方括号首页后缀。移动端保存限制须另行核验并记录受限范围，不能强制套用到PC，也不得规避登录、付费或鉴权要求；
3. **🔴 铁律 3 (RULE-LAZY-RULE-PROTOCOL)**：选集必须使用标准 `@lazyRule=.js:(() => { return playUrl; })()` 闭包协议，绝对禁止伪前缀 `lazy:`；
4. **🔴 铁律 4 (RULE-HOTLINK-GUARD)**：仅转发目标实际要求的媒体或图片请求头；原接口headers为空时保持为空，不强加Referer或User-Agent=PC。添加错误请求头同样会造成403；
5. **🔴 铁律 5 (RULE-SERIES-NAME-GUARD)**：二级详情页首个卡片必须显式注入 `title: '剧名：' + cleanTitle`，保护播放历史不退化；
6. **🔴 铁律 6 (RULE-INTEGER-VERSION)**：版本号 `version` 必须为正整数（如 `1`, `2`），严禁使用字符串；
7. **🔴 铁律 7 (RULE-NO-COMMA-SELECTOR)**：选择器严禁使用英文逗号 `,`，防止移动端 MyJsoup 触发 `NumberFormatException` 原生崩溃；
8. **🔴 铁律 8 (RULE-ISOMORPHIC-VARS)**：二级页面及各级函数首行统一使用双端安全变量垫片：
   `var currentUrl = (typeof MY_URL !== 'undefined' && MY_URL) ? MY_URL : (typeof input !== 'undefined' ? input : '');`；
9. **🔴 铁律 9 (RULE-MULTI-TIER-TRAVERSAL)**：复杂多季/多线路采用模式 A（`@rule=` 级联路由）与模式 B（二次异步接口合并），深层页面持续贯穿父剧名。

---

## 五、全生命周期工具箱 CLI (含质量深度评分与局域网热推)

| 工具脚本 | 核心功能 | 命令行示例 |
|---|---|---|
| `validate-rule.cjs` | 静态同构审计（违禁域名、裸DOM、防盗链、剧名卡片、变量垫片） | `node scripts/validate-rule.cjs my-rule.json` |
| `test-rule.cjs` | 本地沙箱仿真测试 + **5 星质量评分**（条目丰富度、真源流媒体、图集瀑布流） | `node scripts/test-rule.cjs my-rule.json` |
| `export-command.cjs` | 授权后发布/推送；禁止未授权自动外发 | `node scripts/export-command.cjs my-rule.json` |
| `import-rule.cjs` | 多模态导入（短口令、云链接、Base64口令一键反解为规则 JSON） | `node scripts/import-rule.cjs "云6oooole/xxxxxx/id" out.json` |
| `patch-rule.cjs` | 规则动态微调（修改标题、版号自增、换布局、改防盗链） | `node scripts/patch-rule.cjs in.json out.json --bump-version` |
| `quick-scaffold.cjs` | 快速骨架脚手架（支持 vod, json, comic, drama, cascade） | `node scripts/quick-scaffold.cjs "名称" "URL" cascade` |

---

## 六、标准极简交付规范 (Standard Delivery Format)

交付真实本地文件、源码、依赖、启动/导入方法及逐项验收结果。先说明已验证平台和抽样范围，再列未完成项；没有物理文件或生产运行验证时不得声明规则可用。

- 静态检查和仿真评分只用于发现问题，不能代替生产规则运行库、真实数据和媒体解码。
- Windows 本地桥接依赖必须明确；PC 通过不等于 Android 独立运行通过。
- 已有探针/模板的复用来源必须披露，新规则和必要适配代码由当前智能体实际编写。
- 发布短口令、上传文件及局域网推送只在用户授权范围内进行；默认交付本地产物。未实际上传不得输出示例口令冒充结果。
- 可用性必须绑定目标、产物哈希、运行时和验证记录。出现失败时保留证据并修复，不改成同名替代网站。
