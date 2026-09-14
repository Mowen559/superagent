use serde::{Deserialize, Serialize};
use std::time::Instant;

/// 黄金基准测试用例定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenBenchmarkCase {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub description: String,
    pub sample_prompt: String,
    pub mock_code: String,
    pub required_patterns: Vec<String>,
    pub forbidden_patterns: Vec<String>,
}

/// 单个测试用例评测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkCaseResult {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub passed: bool,
    pub missing_required: Vec<String>,
    pub matched_forbidden: Vec<String>,
    pub execution_time_ms: u128,
    pub error_message: Option<String>,
}

/// 全套基准评测报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub total_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub pass_rate: f64,
    pub total_duration_ms: u128,
    pub results: Vec<BenchmarkCaseResult>,
    pub markdown_summary: String,
}

/// 获取 DeepCloud 内置的 5 大黄金架构基准用例
pub fn get_default_golden_cases() -> Vec<GoldenBenchmarkCase> {
    vec![
        // 用例 1: 数据采集任务 DTO 架构规范
        GoldenBenchmarkCase {
            id: "case_spider_job_dto".to_string(),
            name: "Spider Job Command DTO Contract".to_string(),
            domain: "engine-service".to_string(),
            description: "验证采集任务Command/DTO符合DDD架构规范：包含Validation注解且禁止使用JPA实体注解".to_string(),
            sample_prompt: "编写创建采集任务的 CreateJobCmd DTO，包含任务名称、数据源与任务配置，并遵守 DeepCloud 规范".to_string(),
            mock_code: r#"
package com.deepcloud.engine.domain.dto;

import jakarta.validation.constraints.NotBlank;
import jakarta.validation.constraints.NotNull;
import com.deepcloud.engine.domain.spec.SpiderJobSpec;

public record CreateJobCmd(
    @NotBlank(message = "任务名称不能为空") String name,
    @NotBlank(message = "数据源不能为空") String source,
    @NotNull(message = "采集规格不能为空") SpiderJobSpec spec
) {}
"#.trim().to_string(),
            required_patterns: vec![
                "@NotBlank".to_string(),
                "@NotNull".to_string(),
                "SpiderJobSpec".to_string(),
                "record CreateJobCmd".to_string(),
            ],
            forbidden_patterns: vec![
                "@Entity".to_string(),
                "@Table".to_string(),
                "javax.persistence".to_string(),
                "jakarta.persistence".to_string(),
            ],
        },

        // 用例 2: Controller 层与统一响应契约规范
        GoldenBenchmarkCase {
            id: "case_controller_contract".to_string(),
            name: "Controller API & Uniform Result Contract".to_string(),
            domain: "system-service/common".to_string(),
            description: "验证 Controller 必须继承 BaseController，统一返回 Result<T>，使用 @RequirePermission 鉴权，严禁手写 Map 包装 code: 200".to_string(),
            sample_prompt: "编写 SpiderJobController 的创建接口，遵守 DeepCloud 台账规范".to_string(),
            mock_code: r#"
package com.deepcloud.engine.api;

import com.deepcloud.common.api.BaseController;
import com.deepcloud.common.api.Result;
import com.deepcloud.common.security.annotation.RequirePermission;
import jakarta.validation.Valid;
import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/api/v1/engine/jobs")
public class SpiderJobController extends BaseController {
    private final SpiderJobAppService jobAppService;

    public SpiderJobController(SpiderJobAppService jobAppService) {
        this.jobAppService = jobAppService;
    }

    @PostMapping
    @RequirePermission("engine:job:create")
    public Result<Long> createJob(@Valid @RequestBody CreateJobCmd cmd) {
        return Result.ok(jobAppService.create(cmd));
    }
}
"#.trim().to_string(),
            required_patterns: vec![
                "@RequirePermission(\"engine:job:create\")".to_string(),
                "Result<Long>".to_string(),
                "extends BaseController".to_string(),
                "Result.ok(".to_string(),
            ],
            forbidden_patterns: vec![
                "result.put(\"code\", 200)".to_string(),
                "result.put(\"code\", \"200\")".to_string(),
                "public SpiderJob createJob".to_string(),
                "HashMap<String, Object>".to_string(),
            ],
        },

        // 用例 3: MyBatis-Plus LambdaQueryWrapper 与禁止 JPA
        GoldenBenchmarkCase {
            id: "case_mybatis_lambda_wrapper".to_string(),
            name: "MyBatis-Plus Lambda Query Safety".to_string(),
            domain: "iam-service/engine-service".to_string(),
            description: "验证数据库访问使用 LambdaQueryWrapper，禁止硬编码字段名字符串，严禁使用 JPA".to_string(),
            sample_prompt: "编写查询状态为 ACTIVE 且按创建时间降序的采集任务查询方法".to_string(),
            mock_code: r#"
package com.deepcloud.engine.infrastructure.repository;

import com.baomidou.mybatisplus.core.conditions.query.LambdaQueryWrapper;
import com.deepcloud.engine.infrastructure.persistence.SpiderJobDO;
import java.util.List;

public class SpiderJobRepositoryImpl {
    private final SpiderJobMapper jobMapper;

    public SpiderJobRepositoryImpl(SpiderJobMapper jobMapper) {
        this.jobMapper = jobMapper;
    }

    public List<SpiderJobDO> findActiveJobs() {
        return jobMapper.selectList(
            new LambdaQueryWrapper<SpiderJobDO>()
                .eq(SpiderJobDO::getStatus, "ACTIVE")
                .orderByDesc(SpiderJobDO::getCreateTime)
        );
    }
}
"#.trim().to_string(),
            required_patterns: vec![
                "LambdaQueryWrapper<SpiderJobDO>".to_string(),
                "SpiderJobDO::getStatus".to_string(),
                "SpiderJobDO::getCreateTime".to_string(),
                "jobMapper.selectList".to_string(),
            ],
            forbidden_patterns: vec![
                "new QueryWrapper<SpiderJobDO>()".to_string(),
                ".eq(\"status\",".to_string(),
                "JpaRepository".to_string(),
                "org.springframework.data.jpa".to_string(),
                "Hibernate".to_string(),
            ],
        },

        // 用例 4: 前端 API 解包与请求 Hook 规范
        GoldenBenchmarkCase {
            id: "case_frontend_api_consumption".to_string(),
            name: "Frontend API Consumption & Unwrapping".to_string(),
            domain: "frontend/web".to_string(),
            description: "验证前端调用规范：通过 lib/api 与 services 封装，使用 usePageQuery，禁止手动判断 res.code === '200'，未装 SWR".to_string(),
            sample_prompt: "编写展示爬虫任务列表的前端组件，遵守 02-frontend.mdc 规范".to_string(),
            mock_code: r#"
import React from "react";
import { usePageQuery } from "@/hooks/usePageQuery";
import { pageSpiderJobs } from "@/services/engine";
import { DataTable } from "@/components/ui/data-table";

export function SpiderJobList() {
    const { data, loading, pagination, setPagination } = usePageQuery(pageSpiderJobs, {
        initialFilters: {}
    });

    return (
        <div className="p-4 space-y-4">
            <DataTable data={data?.records || []} loading={loading} pagination={pagination} onPaginationChange={setPagination} />
        </div>
    );
}
"#.trim().to_string(),
            required_patterns: vec![
                "usePageQuery".to_string(),
                "@/services/engine".to_string(),
                "DataTable".to_string(),
            ],
            forbidden_patterns: vec![
                "res.code === \"200\"".to_string(),
                "res.code === 200".to_string(),
                "useSWR".to_string(),
                "from \"swr\"".to_string(),
                "bg-amber-600".to_string(),
            ],
        },

        // 用例 5: LiteFlow EL 规则流与组件定义规范
        GoldenBenchmarkCase {
            id: "case_liteflow_el_rule".to_string(),
            name: "LiteFlow EL Rule & Orchestration".to_string(),
            domain: "engine-service/pipeline".to_string(),
            description: "验证采集引擎 LiteFlow EL 编排规则及组件：使用 @LiteflowComponent，遵守 THEN/IF 编排规范".to_string(),
            sample_prompt: "编写用于数据提取的 LiteFlow NodeComponent 以及对应的 EL 流程编排规则定义".to_string(),
            mock_code: r#"
package com.deepcloud.engine.pipeline.component;

import com.yomahub.liteflow.annotation.LiteflowComponent;
import com.yomahub.liteflow.core.NodeComponent;
import com.deepcloud.engine.pipeline.context.PipelineContext;

@LiteflowComponent("myExtractor")
public class MyExtractComponent extends NodeComponent {
    @Override
    public void process() {
        PipelineContext ctx = this.getContextBean(PipelineContext.class);
        ctx.setRecords(ctx.getRawData());
    }
}
// EL 编排规则: THEN(authMount, proxySelect, IF(apiMode, apiCall, httpLoad), myExtractor, publish)
"#.trim().to_string(),
            required_patterns: vec![
                "@LiteflowComponent(\"myExtractor\")".to_string(),
                "extends NodeComponent".to_string(),
                "this.getContextBean(PipelineContext.class)".to_string(),
                "THEN(".to_string(),
                "IF(".to_string(),
            ],
            forbidden_patterns: vec![
                "Thread.sleep".to_string(),
                "while (true)".to_string(),
            ],
        },

        // 用例 6: HashiCorp Vault 凭据安全存储与解析规范
        GoldenBenchmarkCase {
            id: "case_vault_credentials_safety".to_string(),
            name: "Vault Credential Safety & No Plaintext".to_string(),
            domain: "system-service/iam-service".to_string(),
            description: "凭据存储在 HashiCorp Vault，禁止代码中硬编码明文密码、密钥或数据库凭据".to_string(),
            sample_prompt: "编写读取 SMTP 端点凭据的 Vault 服务调用逻辑，避免明文硬编码".to_string(),
            mock_code: r#"
package com.deepcloud.system.infrastructure.vault;

import org.springframework.vault.core.VaultTemplate;
import org.springframework.vault.support.VaultResponse;

public class VaultSecretService {
    private final VaultTemplate vaultTemplate;

    public VaultSecretService(VaultTemplate vaultTemplate) {
        this.vaultTemplate = vaultTemplate;
    }

    public String resolveSecret(String secretRef) {
        VaultResponse response = vaultTemplate.read(secretRef);
        return response != null ? (String) response.getData().get("password") : null;
    }
}
"#.trim().to_string(),
            required_patterns: vec![
                "VaultTemplate".to_string(),
                "secretRef".to_string(),
                "vaultTemplate.read".to_string(),
            ],
            forbidden_patterns: vec![
                "password = \"123456\"".to_string(),
                "root:password".to_string(),
                "password = \"root\"".to_string(),
                "secret = \"".to_string(),
            ],
        },

        // 用例 7: RabbitMQ 消息幂等消费规范
        GoldenBenchmarkCase {
            id: "case_rabbitmq_idempotent_consumer".to_string(),
            name: "RabbitMQ Idempotent Message Consumer".to_string(),
            domain: "notification-service/social-service".to_string(),
            description: "RabbitMQ 消费端幂等性规范：使用 @RabbitListener，必须基于 messageId 或 clientRequestId 校验幂等消费".to_string(),
            sample_prompt: "编写消费通知事件的 RabbitMQ 监听器，确保消费幂等且无重复派发".to_string(),
            mock_code: r##"
package com.deepcloud.notification.infrastructure.mq;

import com.deepcloud.common.annotation.Idempotent;
import org.springframework.amqp.rabbit.annotation.RabbitListener;
import org.springframework.stereotype.Component;

@Component
public class NotificationConsumer {
    @RabbitListener(queues = "notification.event.queue")
    @Idempotent(keyExpression = "#event.eventId", expireSeconds = 300)
    public void onNotificationEvent(NotificationEvent event) {
        processNotification(event);
    }
}
"##.trim().to_string(),
            required_patterns: vec![
                "@RabbitListener".to_string(),
                "@Idempotent".to_string(),
                "expireSeconds = 300".to_string(),
            ],
            forbidden_patterns: vec![
                "Thread.sleep".to_string(),
                "System.out.println".to_string(),
            ],
        },

        // 用例 8: 自研 PermissionVoter 职责链扩展规范
        GoldenBenchmarkCase {
            id: "case_permission_voter_chain".to_string(),
            name: "Custom PermissionVoter Dimension Extension".to_string(),
            domain: "iam-service/security".to_string(),
            description: "自研 PermissionVoter 职责链扩展规范：实现 PermissionVoter 接口，返回 VoteResult.GRANTED / VoteResult.DENIED / VoteResult.ABSTAIN".to_string(),
            sample_prompt: "实现一个自定义维度的 PermissionVoter 投票器，遵守职责链模式".to_string(),
            mock_code: r#"
package com.deepcloud.iam.security.voter;

import com.deepcloud.common.security.PermissionContext;
import com.deepcloud.common.security.PermissionVoter;
import com.deepcloud.common.security.VoteResult;
import org.springframework.core.annotation.Order;
import org.springframework.stereotype.Component;

@Component
@Order(3)
public class CustomDimensionVoter implements PermissionVoter {
    @Override
    public VoteResult vote(PermissionContext ctx) {
        if (!ctx.getPermission().startsWith("custom:")) {
            return VoteResult.ABSTAIN;
        }
        return checkDimension(ctx) ? VoteResult.GRANTED : VoteResult.DENIED;
    }

    private boolean checkDimension(PermissionContext ctx) {
        return true;
    }
}
"#.trim().to_string(),
            required_patterns: vec![
                "implements PermissionVoter".to_string(),
                "VoteResult.ABSTAIN".to_string(),
                "VoteResult.GRANTED".to_string(),
                "VoteResult.DENIED".to_string(),
                "@Component".to_string(),
                "@Order(3)".to_string(),
            ],
            forbidden_patterns: vec![
                "hasRole(".to_string(),
                "SecurityContextHolder.getContext()".to_string(),
            ],
        },
    ]
}

/// 执行单项基准评测
pub fn evaluate_case(case: &GoldenBenchmarkCase, code: &str) -> BenchmarkCaseResult {
    let start = Instant::now();
    let mut missing_required = Vec::new();
    let mut matched_forbidden = Vec::new();

    // 校验必须匹配的模式
    for req in &case.required_patterns {
        if !code.contains(req) {
            missing_required.push(req.clone());
        }
    }

    // 校验禁止出现的反模式
    for forb in &case.forbidden_patterns {
        if code.contains(forb) {
            matched_forbidden.push(forb.clone());
        }
    }

    let passed = missing_required.is_empty() && matched_forbidden.is_empty();
    let execution_time_ms = start.elapsed().as_millis();

    let error_message = if !passed {
        let mut reasons = Vec::new();
        if !missing_required.is_empty() {
            reasons.push(format!("Missing required patterns: {:?}", missing_required));
        }
        if !matched_forbidden.is_empty() {
            reasons.push(format!("Found forbidden anti-patterns: {:?}", matched_forbidden));
        }
        Some(reasons.join(" | "))
    } else {
        None
    };

    BenchmarkCaseResult {
        id: case.id.clone(),
        name: case.name.clone(),
        domain: case.domain.clone(),
        passed,
        missing_required,
        matched_forbidden,
        execution_time_ms,
        error_message,
    }
}

/// 运行黄金基准测试集并输出完整综合评测报告
pub fn run_golden_benchmark(
    filter: Option<&str>,
    custom_target: Option<(&str, &str)>, // (case_id, custom_code)
) -> BenchmarkReport {
    let start = Instant::now();
    let cases = get_default_golden_cases();
    let mut results = Vec::new();

    for case in &cases {
        // 如果有过滤器且未匹配到 id 或 domain，则跳过
        if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            if !case.id.to_lowercase().contains(&f_lower)
                && !case.domain.to_lowercase().contains(&f_lower)
                && !case.name.to_lowercase().contains(&f_lower)
            {
                continue;
            }
        }

        // 如果用户提供了指定 case 的自定义代码，则评测自定义代码，否则评测黄金标准 mock_code
        let code_to_eval = if let Some((target_id, custom_code)) = custom_target {
            if case.id == target_id {
                custom_code
            } else {
                &case.mock_code
            }
        } else {
            &case.mock_code
        };

        let result = evaluate_case(case, code_to_eval);
        results.push(result);
    }

    let total_cases = results.len();
    let passed_cases = results.iter().filter(|r| r.passed).count();
    let failed_cases = total_cases.saturating_sub(passed_cases);
    let pass_rate = if total_cases > 0 {
        (passed_cases as f64 / total_cases as f64) * 100.0
    } else {
        0.0
    };
    let total_duration_ms = start.elapsed().as_millis();

    // 生成 Markdown 格式报告
    let mut md = String::new();
    md.push_str("# 🏆 DeepCloud Golden Benchmark Evaluation Report\n\n");
    md.push_str(&format!(
        "- **Pass Rate**: `{:.1}%` ({}/{})\n",
        pass_rate, passed_cases, total_cases
    ));
    md.push_str(&format!("- **Total Duration**: `{} ms`\n", total_duration_ms));
    md.push_str(&format!(
        "- **Status**: {}\n\n",
        if failed_cases == 0 {
            "🟢 ALL PASSED"
        } else {
            "🔴 CONFORMANCE VIOLATIONS DETECTED"
        }
    ));

    md.push_str("| Case ID | Name | Domain | Result | Latency | Diagnosis |\n");
    md.push_str("|:---|:---|:---|:---:|:---:|:---|\n");

    for r in &results {
        let status_badge = if r.passed { "✅ PASS" } else { "❌ FAIL" };
        let diagnosis = r
            .error_message
            .as_deref()
            .unwrap_or("Conforms strictly to architecture contracts");
        md.push_str(&format!(
            "| `{}` | {} | `{}` | {} | `{}ms` | {} |\n",
            r.id, r.name, r.domain, status_badge, r.execution_time_ms, diagnosis
        ));
    }

    BenchmarkReport {
        total_cases,
        passed_cases,
        failed_cases,
        pass_rate,
        total_duration_ms,
        results,
        markdown_summary: md,
    }
}

/// 动态工作区健康诊断 (Dynamic Workspace Health Verification)
pub fn verify_workspace_syntax(project_path: &std::path::Path) -> Result<String, String> {
    if project_path.join("Cargo.toml").exists() {
        let output = std::process::Command::new("cargo")
            .arg("check")
            .arg("--tests")
            .current_dir(project_path)
            .output()
            .map_err(|e| format!("Failed to spawn cargo check: {e}"))?;
        if output.status.success() {
            Ok("Cargo check --tests passed: Rust workspace syntax & types verified.".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Cargo check verification failed:\n{stderr}"))
        }
    } else if project_path.join("pom.xml").exists() {
        Ok("Maven workspace detected: Java Spring Boot 3.2 project ready for validation.".to_string())
    } else if project_path.join("package.json").exists() {
        Ok("Node.js/Next.js workspace detected: TypeScript types ready for validation.".to_string())
    } else {
        Ok("Generic project workspace detected.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_golden_cases_all_pass() {
        let report = run_golden_benchmark(None, None);
        assert_eq!(report.total_cases, 8);
        assert_eq!(report.passed_cases, 8);
        assert_eq!(report.failed_cases, 0);
        assert_eq!(report.pass_rate, 100.0);
    }

    #[test]
    fn test_forbidden_detection() {
        let case = &get_default_golden_cases()[1]; // case_controller_contract
        let bad_code = r#"
            public HashMap<String, Object> createJob() {
                HashMap<String, Object> result = new HashMap<>();
                result.put("code", 200);
                return result;
            }
        "#;
        let res = evaluate_case(case, bad_code);
        assert!(!res.passed);
        assert!(res.matched_forbidden.contains(&"result.put(\"code\", 200)".to_string()));
    }
}
