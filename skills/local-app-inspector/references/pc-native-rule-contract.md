# PC生产规则契约：无需重新寻找运行库

适用范围：用户已明确选择原生桥接依赖的PC方案。本页不能把独立JS任务改为本地服务；用户要求独立规则时，继续还原远端接口及必要签名/解密，最终不使用本机端口、DLL或Python。

本页依据项目的`deepcloud-ui/native/runtime/hk-engine.js`中runRule/customContext与`hiker-rule-filters.js`中filterState/expandRuleUrl整理。它描述执行接口，不提供最终业务规则或HTTP桥接实现。

## 入口与数据流

| 阶段 | 规则字段 | 实际输入 | 输出 |
|---|---|---|---|
| 首页 | url、find_rule | MY_URL、MY_PAGE、MY_PARAMS | setResult(卡片数组) |
| 搜索 | search_url、searchFind | MY_URL、MY_KEYWORD/KEYWORD、MY_PAGE | setResult(卡片数组) |
| 详情 | detail_find_rule | MY_URL为点击的详情地址，MY_ITEM为原卡片 | setResult(详情和选集卡片数组) |
| 播放 | 选集url中的@lazyRule=脚本 | input为该标记前的地址，MY_PARAMS保持参数 | 实际播放URL及必要请求头 |

这些脚本使用js:入口；lazy支持.js:表达式。不要把只有一个变量名的字符串当成脚本执行结果。卡片常用title、url、pic_url、desc、col_type；媒体列表可用movie_3，选集可用text_3。

## 分类与分页

- class_name/class_url分别用`&`分隔名称和对应值，运行库可自动生成分类按钮。类别值来自原接口，不能用数组序号代替原ID。
- url中的fyclass、fypage在执行前按本次分类与页码展开。分类来自MY_PARAMS.class或MY_PARAMS.fyclass，缺省才取class_url第一项。
- **MY_CATE不是该PC运行库保证注入的变量**。不要依赖它维持分类；可使用展开后的MY_URL，或上述MY_PARAMS字段。
- 若url没有分类占位符而class_url仅为数字，单纯切换分类不会替你构造新的请求URL。首页脚本必须实际消费选中的分类。
- PC本地桥接基址由MY_PARAMS.bridgeBase提供，缺省可用声明的本地端口。替换基址时保留MY_URL已经展开的路径与查询，不能重新用默认频道覆盖本次channel/page。
- 普通首页和搜索页码都可用MY_PAGE；搜索还注入MY_SEARCH_PAGE。search_url中的`**`展开为编码后的关键词。
- 本地API统一支持page=1，无需特殊首页后缀。不要追加普通`[http://首屏地址]`，该PC展开器只专门处理[firstPage=...]；普通方括号会进入请求参数，导致page解析失败。HTML模板仅是异构网站示例，不要复制其选择器或特殊地址到JSON API规则。
- 首页/搜索/详情优先使用MY_URL；这些入口的input可能为空。lazy的input才是选集@lazyRule标记前的实际地址。
- 桥接返回error或JSON字段形状不符时主动报出有界错误，不能用空items掩盖HTTP失败或参数问题。

## 详情、选集和lazy

- 列表/搜索卡片的url应含该项目真实详情ID；详情脚本从当前地址取得ID并请求原目标桥接。
- DMGHG的parts每项有play、play_zh及part字符串数组。保留原选集字符串，生成的播放请求要同时传id、part、play，桥接调用video.play时resolve=true。
- 将lazy函数源码与参数分别序列化，避免在JSON、JS和Shell间叠加手工转义。复杂生成脚本先用write_to_file落盘再执行。
- 必须检查详情实际返回的选集url中的lazy脚本；外层JSON合法不代表多次求值后仍合法。尤其是字符串中的正则反斜杠可能被提前消费，令最终lazy出现意外注释。以函数toString及JSON.stringify保留边界，最后执行真实lazy确认。
- video.play返回的headers为空就保持为空，不能强加Referer或User-Agent=PC。不得把一次取得的签名地址硬编码进规则，也不要把这些地址打印进日志。

## 交付与验证顺序

1. 已知探针只核对原生协议和数据。先完成三个要求的交付文件，桥接只依赖标准库和已声明的安装资源。
2. 调用verify_delivery检查全部产物；该工具执行宿主固定验证器，不需要你定位、阅读或重写测试器。缺文件不是测试器缺依赖。
3. 验证返回具体错误后只修正相关产物，再验证。若建立了plan_task，使用真实收据完成对应进度；最后留出轮次交付结论。
4. README区分功能映射与实际验收：独立裁判抽查两分类、两个关键词第一页、默认列表两页、详情前中后选集名称及首集解码。不要将这些样本写成所有频道/所有分页/所有剧集均已测试。

本流程只认证无界面的PC运行库及Windows本地依赖。Android实机、前端窗口、发布与设备推送各自需要另外的证据或授权。
