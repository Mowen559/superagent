# 移动端与 PC 端反爬防御攻防与风控自愈手记

> 本手记沉淀自 DeepCloud 核心契约台账（#200 ~ #220），针对移动端和 PC 端桌面运行环境中经常遇到的各类反爬盾、防盗链拦截与播放死锁提供成熟处置模式。

---

## 一、宝塔盾 / Nginx WAF / Cloudflare 滑块与人机核验

### 1.1 现象与特征
- 页面返回状态码 `403 Forbidden`、`502 Bad Gateway` 或 `200 OK`，但内容是一段携带 `bt_protect`、`cloud_waf`、`cf_clearance` 的混淆 JS 或带滑块的 HTML；
- 返回正文中含有：`<script>setTimeout(function(){ ... window.location.reload() }, ...)</script>`。

### 1.2 双端同构自愈解决方案
1. **轻量会话复用（Session Mirroring）**：
   - 如果站点仅要求一次滑动验证（通常下发 `PHPSESSID` 或 `waf_cookie`），优先提示或自动呼起 WebView 验证一次；
   - 验证成功后，通过 `getItem("cookie_pool")` 持久化，后续请求直接带上 Cookie，避免每页都弹盾。
2. **嗅探器共享会话（Persist Hiker Session）**：
   - 规则播放地址注入：`url + ';{Cookie@' + getSessionCookie() + '}'`。

---

## 二、图床伪装切片与混淆视频流（假 PNG/JPG 头）

### 2.1 现象与特征（契约台账 #211）
- 站点返回的 M3U8 切片清单中，每一个 `.ts` 文件都是 `.png` 或 `.jpg` 后缀；
- 下载或直接用播放器播放时，报 `Demuxer Stall`、`Unrecognized file format` 或黑屏有声无画；
- 查看该文件的十六进制前几个字节，是合法的 PNG 签名（`89 50 4E 47 0D 0A 1A 0A`），但从第 62 个字节开始才是真实的 MPEG-TS 视频同步字（`0x47`）。

### 2.2 解决方案
- **PC 桌面端**：已由微内核特权本地流代理（`stream-accelerate-proxy.js` / `media-stream-proxy.js`）自动剥离假头；
- **移动端同构规范**：
  在 `lazy` 脚本中，如果探测到目标为伪装切片，优先通过解析接口提取真实纯净 M3U8，或者在 URL 结尾添加特定协议标记 `;fake_png` 通知底层解码器自动跳过前 61 字节偏移。

---

## 三、动态时效 Token 与签名计算逆向

### 3.1 现象
- 播放直链中包含 `sign=md5(url+timestamp+salt)` 或 `auth_key=...`，有效期仅 60 秒到 5 分钟，直接在详情页解析出来的 URL 过期无法播放。

### 3.2 黄金模式：标准 `lazy:` 惰性求值脚本
**严禁在二级页详情列表（`find_rule`）里就提前请求全部选集的播放直链！**
- 错误做法：在解析选集列表时，循环遍历请求 50 次接口获取 50 个直链（导致接口被封，且第一集的直链在播到第十集时已过期）；
- 正确做法：二级页选集只放选集 ID 或页面 URL，将解析脚本放入 `lazy`：
  ```javascript
  // 二级页只输出轻量选集：
  d.push({
    title: epName,
    url: "lazy:" + "https://api.site.com/get_play_url?id=" + epId
  });
  ```
  当且仅当用户实际点击该集起播时，播放引擎才会即时执行该 `lazy` 请求，生成即时有效的高精度 Token，毫秒级起播。

---

## 四、图片防盗链 403 碎裂与全网免死金牌注解

### 4.1 现象
- 移动端海报正常，但在桌面端 Chromium 审查时全量图裂；或者部分移动端也因为 Referer 严格校验返回 403。

### 4.2 双端通用标头注解
海阔规范中，图片防盗链统一在 URL 后追加注解：
```javascript
// 基础防盗链
const finalImg = rawImg + '@Referer=' + encodeURIComponent(homeUrl);

// 无值 Query Flag 语法保留 (针对必须带特定 flag 的 CDN)
const cdnImg = rawImg + '@Referer=https://img.cdn.com/&User-Agent=Mobile';
```
宿主与移动端图片加载器均已内置解析管道，在出网请求图片时会自动剥离 `@` 后缀，注入为 HTTP 请求头，消灭防盗链报错。
