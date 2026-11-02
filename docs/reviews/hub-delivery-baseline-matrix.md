# Hub P0 能力证据矩阵

冻结日期：2026-10-30；源码与二进制身份见 `hub-delivery-p0-baseline.json`。这是现有证据分类，不是新业务通过报告。

| 能力 | 状态与边界 | 依据 |
|---|---|---|
| 原生图谱/项目记忆/正向改码 | 已在合成小工程验证 | [原生能力审计](2026-10-30-hub-native-capability-audit.md) |
| 外部 Understand-Anything | 未认证，metadata 仍缺 | [原生能力审计](2026-10-30-hub-native-capability-audit.md) |
| RTK | 调用已验证；短输出节省为零 | [原生能力审计](2026-10-30-hub-native-capability-audit.md) |
| Skill 发现/资源/合成生成 | 已验证固定夹具，不证明任意规则可用 | [技能审计](2026-10-30-skill-execution-audit.md) |
| DMGHG 原生数据/媒体探针 | 固定版本抽样已验证，探针由宿主预写 | [探针审计](2026-10-30-skill-execution-audit.md#动漫共和国的实际结果) |
| Hub 自写 DMGHG 规则/桥接 | 未验证，作为 P2 必需交付 | [阶段计划](../../openspec/changes/evolve-hub-autonomous-delivery/design.md) |
| SDK/LocalGateway | 无界面组件接线已测，非窗口全流程认证 | [技能审计](2026-10-30-skill-execution-audit.md) |
| 只读纠错复核 | 明确反馈后成功，非独立迁移成绩 | [技能审计](2026-10-30-skill-execution-audit.md#最终接线收据) |
| 大结果读取路径、origin 执行根 | 静态疑点；P1 专项复现，不先声称漏洞已确认 | [阶段设计](../../openspec/changes/evolve-hub-autonomous-delivery/design.md) |

测试计数、组件注册、文件存在均不单独构成业务认证。旧失败保留，首次尝试、辅导和环境问题分别报告。
