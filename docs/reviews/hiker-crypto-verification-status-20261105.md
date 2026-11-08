# 当前规则加解密与验证处理（2026-11-05）

动漫共和国独立JS v2指纹：d6466413b28076fe91e0fd1c65c77e2dea4e671a695baf51c30fe503578a011e。此次只核对并修正技能说明，未改该规则、已有云ID或新建发布。

| 层次 | 当前处理 | 已有证据与边界 |
|---|---|---|
| 动漫共和国请求认证 | 规则内嵌CryptoJS，按已还原协议构造时间戳与AES认证字段 | 实际find_rule的makeRequestHeaders/requestDmgApi；不是AI临时猜参数，也不需Python桥接 |
| 响应解密 | 规则自己的BigInt/RSA实现恢复响应AES密钥，CryptoJS以AES-CBC/PKCS7解密JSON | decryptResponse/rsaDecryptBlock；须按实际JS引擎检查BigInt，不能推广到所有老版本客户端 |
| 播放请求/地址 | 构造随机值、有效期及MD5相关认证，处理加密字段并解析播放候选 | generateAuthParams/resolvePlayUrl/decryptUrlField；部分字段解密失败会回退原文，因此仍需最终地址及真实媒体验证，不能把“不抛异常”当成功 |
| PC通用算法兼容 | CryptoUtil及部分Java接口由宿主兼容层提供，Node crypto与CryptoJS按实现使用 | 本轮AES CBC/ECB、DES回退、摘要等既有测试通过；不是任意Java类、算法或Android完全兼容认证 |
| 图形验证码 | 规则提供验证码图片、输入、真实提交；PC负责卡片/提交联动 | 用户输入，不是自动识别破解；提交成功要看服务端实际结果 |
| 浏览器验证 | 规则/宿主打开验证页面，浏览器执行页面脚本，需交互时由用户完成 | BrowserViewRenderer检测挑战/内容状态并尝试回写会话、同步Cookie、恢复原流程；启发式页面或Cookie出现不证明服务器放行 |
| Cookie复用 | PC监听Electron Session Cookie变化、包含HttpOnly的会话镜像，并由桥接/同步供规则使用 | 当前设备会话可能过期或与环境绑定。手机由海阔自身承担相关能力，PC实现不能替代手机实测 |
| 可选Python/无头浏览器 | 只在用户明确选择有依赖的PC方案时核验并使用 | 动态渲染/stealth不是Cloudflare、Akamai、滑块等通用通过保证；独立JS任务不自动转成桥接 |
| 云口令 | 分发与导入规则的入口 | 不代替接口解密、签名、登录或站点验证 |

本轮离线测试：10项Node测试通过，见[原始日志](D:/抖音/deepcloud/super-agent/artifacts/hub-memory-routing-20261105/v1/crypto-verification-tests.log)。其中包括CryptoUtil算法行为和假会话Cookie镜像测试，也包括验证码提交/验证恢复的静态契约断言；没有操控浏览器、访问实际挑战页面或进行新的真人验证。因此不据此认证当前任意站点过盾。用户此前确认手机上的动漫共和国v2可用，该反馈只归属该规则与用户设备范围。

当前实现入口：[独立规则](D:/抖音/deepcloud/super-agent/artifacts/hub-cloud-publish/dmghg-mobile-v2-20261105-r2/project/rule.json)、[PC算法兼容](D:/抖音/deepcloud/deepcloud-ui/native/hiker-js-core/assets/crypto-java.js)、[会话镜像](D:/抖音/deepcloud/deepcloud-ui/native/core/session-cookie-mirror.js)、[验证页面恢复](D:/抖音/deepcloud/deepcloud-ui/src/components/hiker/BrowserViewRenderer.tsx:575)、[验证码提交](D:/抖音/deepcloud/deepcloud-ui/src/components/common/rule-renderer/RuleTextAndButton.tsx:70)。

本次Skill修改同步了workspace与随包来源的目标段落和新参考页，保留两份正文原有的其他差异；不覆盖用户全局同名旧技能。
