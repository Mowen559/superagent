//! 需求模糊度探测与苏格拉底式主动推敲反问 (Proactive Socratic Inquiry Engine)
//!
//! 核心理念：
//! 1. 杜绝“一句话模糊需求就直接盲目开写”的冒进行为；
//! 2. 多维度剖析架构模糊点（服务边界、持久化方案、通信契约、容错降级）；
//! 3. 严格遵循“查实事实 + A/B 方案权衡矩阵 + 专业推荐选项”的结构化三段式反问规范；
//! 4. 可直接对接 `ask_user` 交互工具或向终端用户输出高质量对齐推敲。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 架构推敲维度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InquiryDimension {
    /// 微服务或模块归属边界未定
    ServiceBoundary,
    /// 数据持久化形态未定（MySQL / Redis / SQLite / 纯内存）
    DataPersistence,
    /// 契约与通信协议未定（REST 同步 / MQ 广播 / WebSocket）
    DeliveryContract,
    /// 权限与安全等级未定（@RequirePermission / 内部网关屏蔽）
    SecurityPermission,
    /// 故障与容错降级策略未定（重试退避 / 熔断降级 / 静默容错）
    FailureTolerance,
}

impl std::fmt::Display for InquiryDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceBoundary => write!(f, "微服务边界与模块归属 (Service Boundary)"),
            Self::DataPersistence => write!(f, "数据持久化存储形态 (Data Persistence)"),
            Self::DeliveryContract => write!(f, "契约规范与通信协议 (Delivery Contract)"),
            Self::SecurityPermission => write!(f, "安全边界与权限模型 (Security & Auth)"),
            Self::FailureTolerance => write!(f, "异常容错与降级策略 (Fault Tolerance)"),
        }
    }
}

/// 推敲方案选项
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InquiryOption {
    pub key: String,
    pub title: String,
    pub description: String,
    pub pros: String,
    pub cons: String,
    pub is_recommended: bool,
}

/// 单个维度的苏格拉底式推敲问题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocraticQuestion {
    pub dimension: InquiryDimension,
    pub title: String,
    pub verified_facts: Vec<String>,
    pub options: Vec<InquiryOption>,
}

/// 结构化主动推敲数据包
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocraticInquiryPayload {
    pub raw_prompt: String,
    pub ambiguity_score: f32, // 0.0 ~ 1.0
    pub missing_dimensions: Vec<InquiryDimension>,
    pub questions: Vec<SocraticQuestion>,
}

impl SocraticInquiryPayload {
    /// 渲染为符合项目 AI 规范的专业三段式推敲文案
    pub fn render_socratic_prompt(&self) -> String {
        let mut out = String::new();
        out.push_str("# 💡 需求架构主动推敲 (Proactive Socratic Inquiry)\n");
        out.push_str(&format!(
            "> **模糊度检测分值**: `{:.2}` / 1.00；检测到 `{}` 个关键技术决策维度未收敛。\n\n",
            self.ambiguity_score,
            self.missing_dimensions.len()
        ));

        for (idx, q) in self.questions.iter().enumerate() {
            out.push_str(&format!("### 问题 {}：{}\n", idx + 1, q.title));
            out.push_str(&format!("**所属维度**: {}\n\n", q.dimension));

            if !q.verified_facts.is_empty() {
                out.push_str("#### ① 【已查实的代码事实】(Verified Facts)\n");
                for fact in &q.verified_facts {
                    out.push_str(&format!("- {}\n", fact));
                }
                out.push('\n');
            }

            out.push_str("#### ② 【技术方案权衡矩阵】(Trade-Off Matrix)\n");
            for opt in &q.options {
                let rec_badge = if opt.is_recommended { " `(强烈推荐)`" } else { "" };
                out.push_str(&format!(
                    "- **【方案 {}】{}**{}\n  - **核心设计**: {}\n  - **优势**: {}\n  - **代价/风险**: {}\n",
                    opt.key, opt.title, rec_badge, opt.description, opt.pros, opt.cons
                ));
            }
            out.push('\n');

            out.push_str("#### ③ 【专业决策与选项】\n");
            for opt in &q.options {
                let rec_mark = if opt.is_recommended { " (推荐)" } else { "" };
                out.push_str(&format!("- [{}] {}{}\n", opt.key, opt.title, rec_mark));
            }
            out.push_str("\n---\n\n");
        }

        out.push_str("👉 **请指示你的期望方案（直接回复如 `A` 或针对特定细节调整），我们将遵循你的决断继续执行。**\n");
        out
    }
}

pub struct SocraticInquiryEngine;

impl SocraticInquiryEngine {
    /// 评估输入需求，识别模糊维度并生成结构化推敲数据包
    pub fn evaluate(prompt: &str, _project_root: Option<&Path>) -> Option<SocraticInquiryPayload> {
        let norm = prompt.to_lowercase();
        let words_count = prompt.chars().count();

        // 纯闲聊、只读查询或极详细的精确命令跳过主动推敲
        if contains_any(&norm, &["查看", "分析", "有哪些", "帮我查", "explain", "where", "list", "show"])
            && !contains_any(&norm, &["实现", "开发", "做个", "新增", "重构", "添加", "编写"])
        {
            return None;
        }

        let mut missing_dimensions = Vec::new();
        let mut ambiguity_score: f32 = 0.0;

        // 1. 服务边界判定
        let has_service = contains_any(
            &norm,
            &[
                "engine", "iam", "system", "social", "media", "model",
                "scraper", "notification", "gateway", "super-agent",
                "采集", "认证", "用户", "权限", "通知", "社交", "媒体", "爬虫"
            ],
        );
        if !has_service {
            missing_dimensions.push(InquiryDimension::ServiceBoundary);
            ambiguity_score += 0.35;
        }

        // 2. 持久化判定：提到记录/存储/数据，但未指定介质
        let mentions_data = contains_any(&norm, &["存", "记录", "保存", "历史", "数据", "表", "store", "save", "data", "history"]);
        let specifies_storage = contains_any(&norm, &["mysql", "redis", "sqlite", "mybatis", "实体", "do表", "内存", "缓存"]);
        if mentions_data && !specifies_storage {
            missing_dimensions.push(InquiryDimension::DataPersistence);
            ambiguity_score += 0.25;
        }

        // 3. 契约协议判定：提到推送/通知/接口，但未指定通信模型
        let mentions_delivery = contains_any(&norm, &["通知", "推送", "下发", "事件", "消息", "接口", "api", "notify", "push", "message"]);
        let specifies_protocol = contains_any(&norm, &["http", "rest", "rabbitmq", "mq", "websocket", "sse", "feign", "ipc"]);
        if mentions_delivery && !specifies_protocol {
            missing_dimensions.push(InquiryDimension::DeliveryContract);
            ambiguity_score += 0.25;
        }

        // 4. 容错与重试判定：提到爬虫/采集/外部调用
        let mentions_external = contains_any(&norm, &["爬虫", "抓取", "采集", "请求外部", "三方", "spider", "scrape", "crawl", "fetch"]);
        let specifies_tolerance = contains_any(&norm, &["重试", "超时", "熔断", "降级", "代理", "风控", "retry", "timeout"]);
        if mentions_external && !specifies_tolerance {
            missing_dimensions.push(InquiryDimension::FailureTolerance);
            ambiguity_score += 0.20;
        }

        // 句子过短惩罚
        if words_count < 25 && !missing_dimensions.is_empty() {
            ambiguity_score += 0.20;
        }

        ambiguity_score = ambiguity_score.min(1.0);

        // 如果模糊度达到 0.40 以上，触发主动推敲机制
        if ambiguity_score < 0.40 || missing_dimensions.is_empty() {
            return None;
        }

        let mut questions = Vec::new();
        for dim in &missing_dimensions {
            match dim {
                InquiryDimension::ServiceBoundary => {
                    questions.push(SocraticQuestion {
                        dimension: *dim,
                        title: "目标落地微服务与架构职责归属确认".to_string(),
                        verified_facts: vec![
                            "当前系统采用标准 DDD 微服务架构，核心业务按领域严格拆分；".to_string(),
                            "`engine-service`: 云端采集控制面与资产中心（Java 21）；".to_string(),
                            "`notification-service`: 统一通知服务，是站内信与邮件唯一事实源；".to_string(),
                            "`scraper-service`: 纯 Python 客户端边缘单文件引擎（本地高频抓取，凭据安全留端）。".to_string(),
                        ],
                        options: vec![
                            InquiryOption {
                                key: "A".to_string(),
                                title: "归属于 `notification-service`（推荐）".to_string(),
                                description: "纳入统一通知服务，复用已发布的邮件/站内信模板与投递策略通道。".to_string(),
                                pros: "符合通知领域边界契约，保证全平台模板与投递审计一致。".to_string(),
                                cons: "需要遵守已发布的模板版本规约，不可绕过模板随意发送正文。".to_string(),
                                is_recommended: true,
                            },
                            InquiryOption {
                                key: "B".to_string(),
                                title: "在调用方业务服务内部闭环扩展".to_string(),
                                description: "在具体业务服务（如 `system-service` 或 `social-service`）内部实现私有通知逻辑。".to_string(),
                                pros: "业务内聚，无需跨服务调用。".to_string(),
                                cons: "违反《系统架构设计》通知边界铁律，造成多服务通知逻辑碎片化。".to_string(),
                                is_recommended: false,
                            },
                        ],
                    });
                }
                InquiryDimension::DataPersistence => {
                    questions.push(SocraticQuestion {
                        dimension: *dim,
                        title: "数据持久化形态与存储介质选型".to_string(),
                        verified_facts: vec![
                            "项目后端 ORM 统一使用 MyBatis-Plus 3.5（严禁 JPA/Hibernate）；".to_string(),
                            "数据库采用 MySQL 8.0 主存储 + Redis 7 高速缓存，端侧为 SQLite。".to_string(),
                        ],
                        options: vec![
                            InquiryOption {
                                key: "A".to_string(),
                                title: "MySQL 表实体持久化 + MyBatis-Plus LambdaQueryWrapper（推荐）".to_string(),
                                description: "新建或扩充 DO 实体类，通过 Flyway 脚本迁移表结构，实现持久化审计。".to_string(),
                                pros: "数据强一致，支持复杂关联统计与历史归档。".to_string(),
                                cons: "需要增加数据库迁移脚本与 Mapper 维护成本。".to_string(),
                                is_recommended: true,
                            },
                            InquiryOption {
                                key: "B".to_string(),
                                title: "Redis 键值缓存 + 自动 TTL 过期".to_string(),
                                description: "直接写入 Redis Hash/String，用于高频读写或临时状态流转。".to_string(),
                                pros: "极高读写性能，无需修改表结构。".to_string(),
                                cons: "不具备跨服务长期归档与复杂条件分页查询能力。".to_string(),
                                is_recommended: false,
                            },
                        ],
                    });
                }
                InquiryDimension::DeliveryContract => {
                    questions.push(SocraticQuestion {
                        dimension: *dim,
                        title: "服务间通信模型与接口契约设计".to_string(),
                        verified_facts: vec![
                            "系统已引入统一消息队列 RabbitMQ 3.x，支持 Topic 广播与 Direct 队列消费；".to_string(),
                            "对外 REST 接口必须返回统一 `Result<T>`（`code` 为 String 格式）。".to_string(),
                        ],
                        options: vec![
                            InquiryOption {
                                key: "A".to_string(),
                                title: "RabbitMQ 异步领域事件驱动（推荐）".to_string(),
                                description: "业务服务发布领域事件，下游异步监听消费，实现完全解耦。".to_string(),
                                pros: "削峰填谷，彻底解耦微服务调用链，保证高吞吐与容错。".to_string(),
                                cons: "最终一致性设计，调试链路相对同步调用略复杂。".to_string(),
                                is_recommended: true,
                            },
                            InquiryOption {
                                key: "B".to_string(),
                                title: "OpenFeign 同步 RPC / REST 接口直连".to_string(),
                                description: "通过 Spring Cloud OpenFeign 客户端发起强一致同步调用。".to_string(),
                                pros: "调用直观、即时拿到返回值。".to_string(),
                                cons: "强耦合，若下游服务抖动或未启动会直接阻塞上游链路。".to_string(),
                                is_recommended: false,
                            },
                        ],
                    });
                }
                InquiryDimension::FailureTolerance => {
                    questions.push(SocraticQuestion {
                        dimension: *dim,
                        title: "端云采集异常容错与降级策略".to_string(),
                        verified_facts: vec![
                            "数据采集采用【端云协同模式】：客户端边缘（纯 Python）安全留端，云端负责公开站点调度；".to_string(),
                            "外部站点存在反爬风控、403 与网络波动风险。".to_string(),
                        ],
                        options: vec![
                            InquiryOption {
                                key: "A".to_string(),
                                title: "指数退避重试 + 代理池自动故障切换（推荐）".to_string(),
                                description: "单次失败触发 3 次指数退避，遇到 403 自动轮换出网代理 IP。".to_string(),
                                pros: "自动化鲁棒性高，显著降低抓取中断率。".to_string(),
                                cons: "需要消耗代理配额，重试会带来额外的响应耗时。".to_string(),
                                is_recommended: true,
                            },
                            InquiryOption {
                                key: "B".to_string(),
                                title: "快速失败 + 存入死信队列 (DLQ) 告警".to_string(),
                                description: "发生阻断立即将任务标记失败并投递至 DLQ 死信通道，等待人工接入。".to_string(),
                                pros: "杜绝无效高频重试被目标站点彻底封禁。".to_string(),
                                cons: "无人值守场景下恢复周期较长，需要人工干预重试。".to_string(),
                                is_recommended: false,
                            },
                        ],
                    });
                }
                _ => {}
            }
        }

        Some(SocraticInquiryPayload {
            raw_prompt: prompt.to_string(),
            ambiguity_score,
            missing_dimensions,
            questions,
        })
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vague_notification_prompt_triggers_socratic_inquiry() {
        let prompt = "帮我弄个通知推送接口，记录一下历史";
        let payload = SocraticInquiryEngine::evaluate(prompt, None);
        assert!(payload.is_some());

        let res = payload.unwrap();
        assert!(res.ambiguity_score >= 0.5);
        assert!(res.missing_dimensions.contains(&InquiryDimension::DeliveryContract));
        assert!(res.missing_dimensions.contains(&InquiryDimension::DataPersistence));

        let rendered = res.render_socratic_prompt();
        assert!(rendered.contains("【已查实的代码事实】"));
        assert!(rendered.contains("【技术方案权衡矩阵】"));
        assert!(rendered.contains("(强烈推荐)"));
    }

    #[test]
    fn test_clear_instruction_skips_inquiry() {
        let clear_prompt = "在 notification-service 中基于 MyBatis-Plus 实现站内信查询接口，使用 Result<PageVO> 返回";
        let payload = SocraticInquiryEngine::evaluate(clear_prompt, None);
        // 服务明确、存储明确、契约明确，不应触发强制拦截
        assert!(payload.is_none());
    }

    #[test]
    fn test_read_only_inquiry_skips() {
        let query_prompt = "查看一下当前有哪些微服务和接口定义";
        let payload = SocraticInquiryEngine::evaluate(query_prompt, None);
        assert!(payload.is_none());
    }
}
