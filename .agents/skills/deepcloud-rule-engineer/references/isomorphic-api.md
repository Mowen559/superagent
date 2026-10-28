# 海阔视界 (HikerView) 移动端与 PC 端双端同构 API 规范手册

> **核心原则**：一份规则代码，在 Android 原生海阔视界客户端与 DeepCloud 桌面端（Electron + V8 沙箱）均必须 100% 运行正常，绝不依赖平台特有私有实现。

---

## 一、双端同构核心 API 清单

### 1.1 网络请求类 API

| 方法与签名 | 返回值 | 移动端行为 | PC 桌面端行为 | 同构规范与禁忌 |
|---|---|---|---|---|
| `request(url, options)` | `string` | 原生 OkHttp 同步请求 | 主进程安全特权请求（免 CORS） | **黄金推荐**。统一通过 `headers` 注入 `User-Agent` 与 `Referer`。 |
| `fetch(url, options)` | `Promise<Response>` | 兼容异步 Promise | 类似 Fetch API | 支持流式，但海阔规则中推荐直接用同步 `request` 简化代码。 |
| `post(url, data, headers)` | `string` | 发起 POST 请求 | 发起 POST 请求 | 统一使用对象或 `key=val&key2=val2` 格式传参。 |
| `batchFetch(tasks)` | `Array<string>` | 并行发起多请求 | 并行发起多请求 | 必须限制并发数（<= 8），防止被目标源封 IP。 |

> [!CAUTION]
> **严禁在规则代码中直接调用浏览器裸 `window.fetch` 或 `XMLHttpRequest`**：移动端无真实 DOM 窗口上下文，会直接导致 `ReferenceError: window is not defined`。

---

### 1.2 HTML/DOM 解析类 API (MyJsoup / CSS 选择器)

| 方法 | 说明 | 示例 | 移动端兼容性 | PC 端兼容性 |
|---|---|---|---|---|
| `pdfh(html, selector)` | 解析单项文本或属性值 | `pdfh(html, "a&&href")` | 100% 支持 | 100% (Cheerio垫片) |
| `pdfa(html, selector)` | 解析列表数组节点 | `pdfa(html, ".list&&li")` | 100% 支持 | 100% (Cheerio垫片) |
| `pdfl(html, listSel, titleSel, imgSel, urlSel)` | 快捷批处理列表 | `pdfl(html, ".item", "h3&&Text", "img&&src", "a&&href")` | 100% 支持 | 100% 支持 |
| `pd(html, selector, baseUrl)` | 解析并自动根据 baseUrl 补全相对路径 | `pd(html, "img&&src", "https://site.com")` | 100% 支持 | 100% 支持 |

#### 选择器常用修饰符
- `&&Text`：提取纯文本内容（自动过滤子 HTML 标签）；
- `&&Html`：提取节点内部完整 HTML；
- `&&attr_name`：提取指定属性（如 `&&href`、`&&src`、`&&data-original`）；
- `&&Text.split("：")[1]`：支持内联数组切分（需防御性加 `?.`）。

> [!CAUTION]
> **致命陷阱：绝对禁止在 `pdfa` / `pdfh` 中使用逗号 `,` 复合选择器！**
> - ❌ 致命错误代码：`var items = pdfa(html, ".card, .list_item, .figure");`
> - **移动端崩溃根因**：海阔视界 Android 原生 MyJsoup 封装支持切片索引语法（如 `pdfa(html, "li, 0")`）。当选择器字符串中存在逗号时，底层会误将逗号后面的内容当作整型数字去执行 `Integer.parseInt(".list_item")`，直接抛出原生崩溃异常：
>   `JavaException: java.lang.NumberFormatException: For input string: ".list_item"`！
> - ✅ 正确修复方案：
>   1. **分步降级提取**：
>      ```javascript
>      var items = pdfa(html, ".card");
>      if (!items || items.length === 0) items = pdfa(html, ".list_item");
>      ```
>   2. **内联 JSON 解析**（主流 SPA / SSR 大厂站首选）：直接正则提取 HTML 中内嵌的 JSON 数据，完全绕开 DOM 选择器。

---

### 1.3 存储与跨会话状态 (Storage)

| 方法 | 说明 | 示例 |
|---|---|---|
| `setItem(key, value)` | 本地持久化 KV 存储（同构级持久化） | `setItem("token", "xyz");` |
| `getItem(key, defaultValue)` | 获取本地持久化值 | `const token = getItem("token", "");` |
| `clearItem(key)` | 清除指定的 KV | `clearItem("token");` |

> [!IMPORTANT]
> - `setItem` 存入对象时必须手动 `JSON.stringify`，取出必须安全 `JSON.parse`；
> - 严禁在存储中放入大体积二进制、整页原始 HTML（单 key 不宜超过 64KB）。

---

### 1.4 加解密与通用工具

- **Base64**：
  - 编码：`base64Encode(str)`
  - 解码：`base64Decode(str)`
- **MD5**：`md5(str)`
- **AES 解密**：
  - 移动端通常通过 `CryptoJS` 库或全局 `aesDecode(data, key, iv)`；
  - 双端同构最佳实践：规则内联轻量纯 JS 加密算法或统一依赖 `CryptoJS`。

---

## 二、双端直链注解标准（Hotlink & Stream Header Annotation）

在海阔体系中，流媒体播放器和图片加载器需要根据防盗链标头出网。双端共享的官方注解语法规范如下：

```javascript
// 1. 视频直链标准防盗链注解 (用于 lazy 脚本或播放地址)
const finalPlayUrl = rawUrl + ';{Referer@' + baseDomain + '&&User-Agent@' + standardUa + '}';

// 2. 图片直链防盗链注解 (用于二级页封面或搜索海报)
const safeImgUrl = rawImgUrl + '@Referer=' + baseDomain;

// 3. 强制指定解码器或协议头
const mp4Url = rawUrl + ';{User-Agent@PC&&Referer@' + baseDomain + '};mp4';
```

---

## 三、动态提升解算与跨页传参规范

当搜索或二级页需要携带来源上下文（例如所属分类、Token）时，通过 `@` 传参：

```javascript
// 二级页传参规范：url@ruleParams
const targetUrl = detailUrl + '@rule=' + encodeURIComponent(JSON.stringify({
  sourceRule: MY_RULE.title,
  extraHeader: "token-abc"
}));
```
接收端解包：
```javascript
const parts = MY_URL.split('@rule=');
const currentUrl = parts[0];
const params = parts[1] ? JSON.parse(decodeURIComponent(parts[1])) : {};
```


---

## 四、移动端 (Android 原生海阔视界) 与 PC 桌面端 8 大核心鸿沟与必死陷阱自愈指南

在海阔规则生态中，开发者经常遇到**“明明在 PC 端沙箱里测试能跑，一导入手机海阔视界 App 就闪退、白屏或报错”**。其根本原因在于移动端基于 Android 原生 Java 架构（OkHttp + MyJsoup + Mozilla Rhino / WebView），而 PC 端基于 Node.js / Electron 现代环境（Chromium V8 + Cheerio）。两者的运行时和底层安全沙箱存在极大的代际鸿沟。

### 4.1 鸿沟一：正版违禁域名黑名单拦截 (Domain Shielding)

- **现象**：腾讯视频、爱奇艺、优酷、小红书等规则，PC 端正常拉取，移动端点击保存或打开时直接弹出「频道链接包含违禁域名，无法保存，请支持正版」并拒绝加载。
- **根因**：海阔视界为遵循应用商店合规要求，在 Java 底层写死了大厂正版域名的拦截名单（含 `v.qq.com`, `iqiyi.com`, `youku.com`, `bilibili.com`, `mgtv.com`, `xiaohongshu.com` 等）。如果规则的入口 `url` 或 `search_url` 含有这些域名，底层直接截断。
- **自愈方案**：
  必须使用 `hiker://empty#fyclass#fypage[hiker://empty#fyclass]` 虚拟通道规避首帧域名嗅探，真实请求在 `find_rule` 的 JS 逻辑内部由 `request("https://...")` 动态发起。

---

### 4.2 鸿沟二：JS 引擎代际差异 (Mozilla Rhino vs 现代 V8)

- **现象**：PC 端执行完美，移动端在加载脚本的首帧直接抛出 `SyntaxError: Unexpected token .` 或 `SyntaxError: Unexpected token ?`，导致整页脚本被腰斩。
- **根因**：
  - PC 端使用 Node.js / Electron 的 **现代 V8 引擎**，完全支持 ES2024；
  - 移动端海阔大部分版本运行在 **Mozilla Rhino** 环境中（ES5/早期 ES6 规范）。**Rhino 根本不支持 ES2020+ 的可选链 (`?.`) 与空值合并操作符 (`??`)**！
- **自愈方案**：
  - ❌ 禁止：`const title = res?.data?.items?.[0]?.title ?? '默认';`
  - ✅ 正确：`var title = (res && res.data && res.data.items && res.data.items[0] && res.data.items[0].title) ? res.data.items[0].title : '默认';`
  - 所有变量声明尽量使用 `var`，避免在 Rhino 全局上下文多次重入时报 `Redeclaration of const`。

---

### 4.3 鸿沟三：MyJsoup 逗号切片索引陷阱 (NumberFormatException)

- **现象**：PC 端用 `pdfa(html, ".item, .card")` 提取多个类名很方便，移动端抛出 `JavaException: java.lang.NumberFormatException: For input string: ".card"`。
- **根因**：移动端 Java `MyJsoup` 重载了逗号语法用于行索引切片（如 `pdfa(html, "li, 0")` 取首项）。选择器中只要出现逗号，底层会强行把逗号后内容作为整型解析，导致整页崩溃。
- **自愈方案**：
  - ❌ 禁止在选择器中使用逗号：`pdfa(html, ".item, .card")`
  - ✅ 改为逻辑条件降级：
    ```javascript
    var items = pdfa(html, ".item");
    if (!items || items.length === 0) items = pdfa(html, ".card");
    ```

---

### 4.4 鸿沟四：选集与动作触发协议 (@lazyRule= vs 伪协议 lazy:)

- **现象**：PC 端有些播放器能处理 `lazy:https://...`，但在手机端点击选集要么无响应，要么直接报错打不开。
- **根因**：海阔视界移动端原生解析引擎严格识别 `@lazyRule=.js:(() => { return url; })()` 语法契约。如果写成 `url: "lazy:https://..."`，移动端无法识别为惰性执行脚本，而是将其直接丢给 Android 系统播放器（ExoPlayer/IjkPlayer），系统播放器不支持 `lazy:` 协议抛错退出。
- **自愈方案**：
  - 选集跳转必须规范拼接：`url: epUrl + "@lazyRule=.js:(() => { return input; })()"`。

---

### 4.5 鸿沟五：防盗链与底层媒体播放器出网标头注解

- **现象**：PC 端在浏览器沙箱内由于主进程代理可以免 CORS 播放；移动端调用 Android 系统原生播放器拉流时，被目标 CDN 直接返回 403 Forbidden。
- **根因**：移动端播放器是独立的底层进程，不会自动携带规则脚本中的 Cookie 或 Referer。必须通过海阔特定的注解语法，底层播放器才会将 Headers 注入到拉流 Socket。
- **自愈方案**：
  - 视频直链末尾必须拼接：`;{Referer@${domain}&&User-Agent@${ua}}`
  - 图片直链末尾必须拼接：`@Referer=${domain}`

---

### 4.6 鸿沟六：生命周期全局变量命名差异 (MY_URL vs input)

- **现象**：PC 端写 `input` 能跑，移动端二级页报 `ReferenceError: input is not defined`。
- **根因**：移动端不同页面的入参变量名不同：
  - 一级列表页：`MY_URL`, `MY_PAGE`
  - 二级详情页：`MY_URL` (移动端原生 JSEngine 未注入 input)
  - 搜索页：`KEY`, `MY_PAGE`
  - lazy 脚本：`input`
- **自愈方案**：
  在二级页脚本首行必须加双端同构变量垫片：
  ```javascript
  var currentUrl = (typeof MY_URL !== 'undefined' && MY_URL) ? MY_URL : (typeof input !== 'undefined' ? input : '');
  ```

---

### 4.7 鸿沟七：原生 Android RecyclerView 布局测量与 col_type 契约

- **现象**：PC 端列表正常排版，移动端直接显示空白或卡片高度坍塌为 0。
- **根因**：移动端由 Android 原生 RecyclerView 渲染，严重依赖卡片的 `col_type` 计算宽高测量比例。如果缺失 `col_type` 或拼写错误，原生 ViewHolder 无法绑定视图导致白屏。
- **自愈方案**：
  每个返回的视图节点必须明确标明合规的 `col_type`：
  - 影视海报 3 列：`col_type: "movie_3"`
  - 图集大图单列：`col_type: "pic_1_full"`
  - 宽幅图文卡片：`col_type: "card_pic_2"`
  - 选集按钮行：`col_type: "text_4"` 或 `col_type: "flex_button"`

---

### 4.8 鸿沟八：Android Cleartext HTTP 与 TLS 证书链拦截

- **现象**：PC 端能加载的 HTTP 小网站，移动端报 `Cleartext HTTP traffic to xxx not permitted` 或 `SSLHandshakeException`。
- **根因**：Android 9+ 默认开启了严格的明文流量阻断政策；移动端证书信任链弱于桌面操作系统。
- **自愈方案**：
  优先采用 HTTPS；对于弱证书源，在请求时显式指定 User-Agent，必要时通过 PC 边缘引擎中转。
