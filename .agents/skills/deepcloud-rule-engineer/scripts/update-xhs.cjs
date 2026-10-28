const fs = require('fs');
const path = require('path');

const targetPath = path.resolve(__dirname, '../templates/xiaohongshu.rule.json');
const rule = JSON.parse(fs.readFileSync(targetPath, 'utf8'));

rule.detail_find_rule = `js:
var d = [];
var currentUrl = (typeof MY_URL !== 'undefined' && MY_URL) ? MY_URL : (typeof input !== 'undefined' ? input : '');
var base = 'https://' + 'www.xiaohongshu.com';

// 1. 从 URL payload 中反解出笔记元数据（零延迟首屏呈现）
var meta = {};
var payloadMatch = currentUrl.match(/#payload=([^&]+)/);
if (payloadMatch) {
  try {
    meta = JSON.parse(decodeURIComponent(payloadMatch[1]));
  } catch (e) {}
}

var noteTitle = meta.title || '小红书笔记详情';
var author = meta.author || '小红书精选创作者';
var liked = meta.liked || '';
var cover = meta.cover || '';
var noteId = meta.id || (currentUrl.match(/\\/explore\\/([a-zA-Z0-9]+)/) ? currentUrl.match(/\\/explore\\/([a-zA-Z0-9]+)/)[1] : '');
var isVideo = (meta.type === 'video');

// 详情页首项：注入大剧名/作品名契约保护（铁律 5）
d.push({
  title: '剧名：' + noteTitle,
  desc: '作者：' + author + (liked ? ('   ' + liked) : ''),
  pic_url: cover ? cover + '@Referer=' + base + '/' : '',
  url: currentUrl,
  col_type: 'pic_1_card'
});

// 2. 作者与信息卡片
d.push({
  title: '创作者：' + author,
  desc: (meta.avatar ? '已通过认证' : '小红书优秀创作者') + (liked ? (' | ' + liked) : ''),
  pic_url: (meta.avatar ? meta.avatar + '@Referer=' + base + '/' : ''),
  url: currentUrl,
  col_type: 'text_1'
});

// 若为视频笔记，显式注入免嗅播放卡片 (移动端与 PC 双端通杀)
if (isVideo) {
  var videoPlayUrl = (base + '/explore/' + noteId + (meta.xsecToken ? '?xsec_token=' + meta.xsecToken : '')) + '#isVideo=true##noLoading#';
  d.push({
    title: '▶️ 点击立即免嗅播放视频笔记',
    desc: '海阔硬件加速秒播 | 点击直达',
    pic_url: cover ? cover + '@Referer=' + base + '/' : '',
    url: videoPlayUrl + '@lazyRule=.js:(() => { return input; })()',
    col_type: 'text_1'
  });
}

// 3. 高清原图图集提取 (原图去水印 + 端机协同无头渲染)
var imageList = [];
if (cover) {
  var rawCover = cover.replace(/!nc_n_[^\"'\\s]+/, '').replace(/!nc_[^\"'\\s]+/, '');
  imageList.push(rawCover);
}

if (noteId && meta.xsecToken) {
  var discoveryUrl = base + '/discovery/item/' + noteId + '?xsec_token=' + meta.xsecToken;
  try {
    var html = '';
    var localEngine = 'http://127.0.0.1:18000';
    var isEngineOnline = false;
    try {
      var ping = request(localEngine + '/api/v1/datasources', { timeout: 300 });
      if (ping && ping.indexOf('code') > -1) { isEngineOnline = true; }
    } catch(pe) {}

    if (isEngineOnline) {
      var sRes = post(localEngine + '/api/v1/scrape', {
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url: discoveryUrl, mode: 'stealthy', timeout: 8000 })
      });
      var sJson = JSON.parse(sRes || '{}');
      html = (sJson.data && sJson.data.content) ? sJson.data.content : '';
    } else {
      html = request(discoveryUrl, {
        headers: {
          'User-Agent': 'Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1',
          'Referer': base + '/explore'
        }
      });
    }

    var match = html.match(/__INITIAL_STATE__\\s*=\\s*({[\\s\\S]*?})<\\/script>/);
    if (match) {
      var data = JSON.parse(match[1].replace(/undefined/g, 'null'));
      var noteDetailMap = (data.note && data.note.noteDetailMap) ? data.note.noteDetailMap : {};
      var nObj = noteDetailMap[noteId] ? noteDetailMap[noteId].note : (data.note ? data.note.firstNote : null);
      if (nObj) {
        if (nObj.desc) {
          d.push({
            title: '笔记正文：',
            desc: nObj.desc,
            url: currentUrl,
            col_type: 'text_1'
          });
        }
        if (nObj.imageList && nObj.imageList.length > 0) {
          imageList = [];
          for (var k = 0; k < nObj.imageList.length; k++) {
            var rawPic = nObj.imageList[k].urlDefault || nObj.imageList[k].url;
            if (rawPic) {
              imageList.push(rawPic.replace('http://', 'https://').replace(/!nc_n_[^\"'\\s]+/, ''));
            }
          }
        }
        if (nObj.video && nObj.video.media && nObj.video.media.stream) {
          var stream = nObj.video.media.stream;
          var videoUrl = (stream.h264 && stream.h264[0]) ? stream.h264[0].masterUrl : (stream.h265 ? stream.h265[0].masterUrl : '');
          if (videoUrl) {
            d.push({
              title: '🎬 官方高清直链播放',
              desc: '最高画质流媒体直出',
              url: videoUrl + '@lazyRule=.js:(() => { return input + \";{Referer@' + base + '/&&User-Agent@PC}\"; })()',
              col_type: 'text_1'
            });
          }
        }
      }
    }
  } catch (err) {}
}

// 4. 多图连续瀑布流展现 (海阔原生画廊 pic_1_full，点击即可放大双指缩放)
d.push({
  title: '高清原图画集 (' + imageList.length + 'P 点击放大)：',
  col_type: 'text_1'
});

for (var p = 0; p < imageList.length; p++) {
  var pic = imageList[p];
  d.push({
    title: '图 ' + (p + 1) + ' / ' + imageList.length,
    pic_url: pic + '@Referer=' + base + '/',
    url: pic + '@Referer=' + base + '/',
    col_type: 'pic_1_full'
  });
}

// 5. 移动端无痕内置浏览器直达保底通道 (在遇服务端重度风控时，100% 顺滑滑动阅览全部多图与评论)
d.push({
  title: '🌐 在海阔内置无痕浏览器中滑动查看原图集与评论',
  desc: '无痕环境 | 避开风控 | 支持全图横滑',
  url: 'input://' + base + '/explore/' + noteId + (meta.xsecToken ? '?xsec_token=' + meta.xsecToken : ''),
  col_type: 'text_1'
});

setResult(d);
`;

fs.writeFileSync(targetPath, JSON.stringify(rule, null, 2), 'utf8');
console.log('Successfully updated xiaohongshu.rule.json');
