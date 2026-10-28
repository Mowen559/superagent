# 主流 CMS 特征库与选择器秒查速查手册 (CMS Archetypes)

> 在为未知站点生成海阔视界双端同构规则时，85% 以上的目标站点均基于以下经典 CMS 或模板二次开发。
> 快速识别特征签名，可直接复用成熟选择器与播放器配置提取逻辑。

---

## 1. 苹果CMS (MacCMS v10) / 海螺模板 / 首途模板

### 1.1 核心特征签名
- 页面含有全局变量：`var maccms = {"path":"","mid":"","url":...};`
- 播放页内联播放器配置：`<script type="text/javascript">var player_aaaa={...}</script>`
- 常用 CSS 选择器集合：
  - 列表容器：`.module-items`, `.hl-list-item`, `.stui-vodlist`, `.myui-vodlist`
  - 标题节点：`.module-item-title`, `.hl-item-title`, `h4.title a`, `.stui-vodlist__title`
  - 海报图片：`img&&data-original`, `img&&data-src`, `img&&src`
  - 剧集选集：`.module-blocklist a`, `.hl-plays-list a`, `.stui-content__playlist a`

### 1.2 极速解密模式 (Lazy)
```javascript
var playerMatch = html.match(/player_aaaa\s*=\\s*({[\s\S]*?})<\/script>/);
if (playerMatch) {
  var cfg = JSON.parse(playerMatch[1]);
  var rawUrl = cfg.url; // 真实播放直链或解析二次接口
}
```

---

## 2. 飞飞CMS (FeiFeiCMS) / 光芒CMS

### 2.1 核心特征签名
- 列表包含：`.ff-vod-list`, `.vod-item`, `.play-box`
- 播放配置：`var cms_player = {...};`
- 搜索接口路径通常为：`/vod-search-wd-**.html`

---

## 3. WordPress 影视主题 (如 Bento / Filmio / Vieu)

### 3.1 核心特征签名
- 列表容器：`.films-grid`, `.entry-media`, `.post-box`
- 文章分类：`/category/xxx`, 翻页格式 `/category/xxx/page/fypage/`
- 视频直链：常常直接内嵌 HTML5 `<video src="...">` 或带有 `data-src` 的 iframe

---

## 4. 常见的短剧剧场 CMS (DuanJu CMS)

### 4.1 核心特征签名
- 页面集数众多（50~150集），单集通常为 1~2 分钟
- 选集容器：`.drama-playlist`, `.episodes-box`, `.grid-cols-4`
- 直链类型：大多为直连 `.mp4` 且需要带移动端 User-Agent 与 Referer 注解：
  ```javascript
  playUrl + ';{Referer@' + input + '&&User-Agent@Mobile};mp4'
  ```

---

## 5. 跨平台防盗链注解总表

| 资源类型 | 场景 | 注解语法 | 示例 |
|---|---|---|---|
| **视频流 (HLS/M3U8)** | 带有 Host/Referer 防护 | `;{Referer@<url>&&User-Agent@<ua>}` | `url + ';{Referer@' + input + '&&User-Agent@PC}'` |
| **视频直链 (MP4/FLV)** | 需要强制指定解码后缀 | `;{...};mp4` | `url + ';{Referer@' + input + '};mp4'` |
| **海报封面图片** | CDN 防盗链返回 403 | `@Referer=<url>` | `imgUrl + '@Referer=' + input` |
| **带 Cookie 鉴权图片** | 需要登录态或过盾 Cookie | `@Referer=<url>&Cookie=<cookie>` | `imgUrl + '@Referer=' + input + '&Cookie=' + cookie` |
