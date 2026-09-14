//! 确定性语言规则嗅探与审查规范库 (Language Sniffer & Builtin Ruleset)
//!
//! 借鉴 Alibaba open-code-review 生产级架构设计：
//! 1. 自动嗅探变更文件涉及的编程语言与项目框架类型；
//! 2. 注入针对特定语言与高危领域的确定性缺陷防范规则（Safety / Security / Logic / Performance）；
//! 3. 避免大模型漫无目的地随机审查，强制对齐工业级质量红线（如 NPE 链、并发竞争、资源泄露、SQL 注入等）。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportedLanguage {
    Java,
    Go,
    Rust,
    TypeScript,
    JavaScript,
    Python,
    C,
    Cpp,
    Sql,
    Unknown,
}

impl SupportedLanguage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Sql => "sql",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleCategory {
    Safety,      // 空指针、内存/协程泄露、并发竞态
    Security,    // SQL注入、XSS、凭据泄露、越权
    Logic,       // 死循环、死代码、逻辑短路、条件边界
    Performance, // N+1 查询、热点循环内重度开销、无界缓存
}

impl RuleCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Safety => "安全与健壮性 (Safety)",
            Self::Security => "漏洞与防御 (Security)",
            Self::Logic => "代码逻辑正确性 (Logic)",
            Self::Performance => "运行时性能 (Performance)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRule {
    pub id: String,
    pub language: SupportedLanguage,
    pub category: RuleCategory,
    pub title: String,
    pub description: String,
    pub pattern_hint: Option<String>,
}

/// 语言嗅探器 (Language Sniffer)
pub struct LanguageSniffer;

impl LanguageSniffer {
    /// 根据文件路径与扩展名推断语言
    pub fn sniff_path(path: &Path) -> SupportedLanguage {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "java" => SupportedLanguage::Java,
            "go" => SupportedLanguage::Go,
            "rs" => SupportedLanguage::Rust,
            "ts" | "tsx" => SupportedLanguage::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => SupportedLanguage::JavaScript,
            "py" | "pyw" => SupportedLanguage::Python,
            "c" => SupportedLanguage::C,
            "cpp" | "cc" | "cxx" | "hpp" => SupportedLanguage::Cpp,
            "h" => {
                // .h 头文件进行内容嗅探
                if let Ok(content) = std::fs::read_to_string(path) {
                    if content.contains("template<") || content.contains("class ") || content.contains("namespace ") {
                        return SupportedLanguage::Cpp;
                    }
                }
                SupportedLanguage::C
            }
            "sql" => SupportedLanguage::Sql,
            _ => SupportedLanguage::Unknown,
        }
    }

    /// 根据首行 Shebang 嗅探脚本类型
    pub fn sniff_shebang(first_line: &str) -> Option<SupportedLanguage> {
        let line = first_line.trim();
        if !line.starts_with("#!") {
            return None;
        }
        if line.contains("python") {
            Some(SupportedLanguage::Python)
        } else if line.contains("node") || line.contains("deno") || line.contains("bun") {
            Some(SupportedLanguage::JavaScript)
        } else {
            None
        }
    }
}

/// 内置确定性规则库 (Builtin Ruleset)
pub struct BuiltinRuleset;

impl BuiltinRuleset {
    pub fn get_rules_for_language(lang: SupportedLanguage) -> Vec<ReviewRule> {
        match lang {
            SupportedLanguage::Java => vec![
                ReviewRule {
                    id: "JAVA_NPE_CHAIN".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "链式调用与未受检对象解引用 (NPE 防范)".into(),
                    description: "多层对象链式取值 (如 a.getB().getC()) 前必须做显式判空或使用 Optional / Null-safe 工具；从 Map/Cache 获取的值必须检查是否为 null。".into(),
                    pattern_hint: Some("get.*\\(\\)\\.get".into()),
                },
                ReviewRule {
                    id: "JAVA_RESOURCE_LEAK".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "IO 与数据库资源显式关闭".into(),
                    description: "InputStream, OutputStream, Connection, ResultSet 必须置于 try-with-resources 块中，严禁在 finally 块外手动关闭。".into(),
                    pattern_hint: Some("new (FileInput|Buffered|Sql)".into()),
                },
                ReviewRule {
                    id: "JAVA_SQL_INJECTION".into(),
                    language: lang,
                    category: RuleCategory::Security,
                    title: "动态 SQL 字符串拼接注入防范".into(),
                    description: "禁止在 MyBatis 或 JDBC 中使用字符串拼接拼装 SQL 条件；XML 映射中参数必须使用 #{} 占位符而非 ${}。".into(),
                    pattern_hint: Some("\\$\\{.*\\}".into()),
                },
                ReviewRule {
                    id: "JAVA_CONCURRENT_THREAD_SAFETY".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "单例/多线程共享状态并发安全".into(),
                    description: "Spring 默认单例 Bean (Controller / Service) 中严禁包含无锁的可变成员变量；SimpleDateFormatter 等非线程安全工具不可作为静态共享成员。".into(),
                    pattern_hint: Some("private SimpleDateFormat".into()),
                },
            ],
            SupportedLanguage::Go => vec![
                ReviewRule {
                    id: "GO_UNCHECKED_ERR".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "未校验的 error 返回值".into(),
                    description: "所有返回 (T, error) 的函数调用必须首先显式检查 if err != nil，严禁直接使用下划线 _ 吞掉关键错误。".into(),
                    pattern_hint: Some("_, _ =".into()),
                },
                ReviewRule {
                    id: "GO_GOROUTINE_LEAK".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "协程泄漏 (Goroutine Leak)".into(),
                    description: "启动 go func() 必须绑定 context.Context 或退出通知通道 (done chan)；严禁在无退出保证的情况下向无缓冲 Channel 永久阻塞写入。".into(),
                    pattern_hint: Some("go func\\(".into()),
                },
                ReviewRule {
                    id: "GO_NIL_MAP_SLICE".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "Nil Map 写入导致 Panic".into(),
                    description: "向 map 写入键值前必须使用 make(map[...]) 进行初始化，向未初始化的 nil map 写入会直接触发 panic。".into(),
                    pattern_hint: Some("var .* map\\[".into()),
                },
                ReviewRule {
                    id: "GO_MUTEX_DEFER_UNLOCK".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "互斥锁成对释放".into(),
                    description: "调用 mu.Lock() 或 RLock() 后紧接着使用 defer mu.Unlock()，防止中途 return 或 panic 造成死锁。".into(),
                    pattern_hint: Some("\\.Lock\\(\\)".into()),
                },
            ],
            SupportedLanguage::Rust => vec![
                ReviewRule {
                    id: "RUST_UNCHECKED_UNWRAP".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "业务路径滥用 .unwrap() / .expect()".into(),
                    description: "在非测试代码中，针对外部 I/O、用户请求和动态解析，严禁裸调 .unwrap()，必须使用 ? 操作符或 match 优雅处理错误。".into(),
                    pattern_hint: Some("\\.unwrap\\(\\)".into()),
                },
                ReviewRule {
                    id: "RUST_HOT_LOOP_CLONE".into(),
                    language: lang,
                    category: RuleCategory::Performance,
                    title: "循环内不必要的重度 .clone()".into(),
                    description: "在遍历迭代中避免不必要的堆分配深拷贝，优先传递引用 (&T / &str) 或使用 Cow/Arc 共享不可变数据。".into(),
                    pattern_hint: Some("for .* in .* \\{.*\\.clone\\(\\)".into()),
                },
                ReviewRule {
                    id: "RUST_DEADLOCK_ORDER".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "多重锁加锁顺序与 Guard 持有生命周期".into(),
                    description: "跨异步 await 边界严禁持有 std::sync::MutexGuard；多个 Lock 同时请求时需确保全局一致的获取次序。".into(),
                    pattern_hint: Some("Mutex::lock".into()),
                },
            ],
            SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => vec![
                ReviewRule {
                    id: "TS_UNHANDLED_PROMISE".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "未捕获的异步 Promise 异常".into(),
                    description: "所有 async 调用必须有 await 配合 try-catch，或结尾显式链式调用 .catch()，避免未捕获异常导致 Node 进程崩溃或前端请求挂起。".into(),
                    pattern_hint: Some("\\.then\\([^.]*\\)$".into()),
                },
                ReviewRule {
                    id: "TS_ANY_ESCAPE".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "滥用 any 绕过核心类型防护".into(),
                    description: "核心业务实体与网络出参禁止使用 any 逃避类型检查，若类型暂不明确需使用 unknown 并在访问属性前进行类型断言 (Type Guard)。".into(),
                    pattern_hint: Some(": any".into()),
                },
                ReviewRule {
                    id: "TS_XSS_INJECTION".into(),
                    language: lang,
                    category: RuleCategory::Security,
                    title: "不安全的 HTML 直出与 XSS 注入".into(),
                    description: "严禁直接使用 dangerouslySetInnerHTML 或 innerHTML 输出未转义的用户输入内容，必须经过 DOMPurify 过滤。".into(),
                    pattern_hint: Some("dangerouslySetInnerHTML".into()),
                },
            ],
            SupportedLanguage::Python => vec![
                ReviewRule {
                    id: "PY_MUTABLE_DEFAULT_ARG".into(),
                    language: lang,
                    category: RuleCategory::Logic,
                    title: "可变对象作为函数默认参数".into(),
                    description: "严禁将 list、dict 或 set 作为函数参数的默认值 (如 def f(x=[]) )，会导致跨调用共享状态污染，应使用 def f(x=None)并在函数体内初始化。".into(),
                    pattern_hint: Some("def .*\\[\\]|\\{\\}".into()),
                },
                ReviewRule {
                    id: "PY_SQL_FSTRING".into(),
                    language: lang,
                    category: RuleCategory::Security,
                    title: "SQL 语句使用 f-string 或 % 拼接".into(),
                    description: "数据库查询必须使用参数化占位符 (cursor.execute(sql, (param,)))，严禁通过 f\"{sql}\" 直接注入动态字符串。".into(),
                    pattern_hint: Some("cursor\\.execute\\(f\"".into()),
                },
            ],
            SupportedLanguage::C | SupportedLanguage::Cpp => vec![
                ReviewRule {
                    id: "C_BUFFER_OVERFLOW".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "不安全字符串函数与缓冲区溢出".into(),
                    description: "严禁使用 strcpy, strcat, sprintf, gets 等无边界检查函数，必须使用 strncpy, snprintf, strncat 或 C++ std::string。".into(),
                    pattern_hint: Some("(strcpy|sprintf|gets)\\(".into()),
                },
            ],
            SupportedLanguage::Sql => vec![
                ReviewRule {
                    id: "SQL_NO_WHERE_UPDATE_DELETE".into(),
                    language: lang,
                    category: RuleCategory::Safety,
                    title: "全表无条件 UPDATE / DELETE 防范".into(),
                    description: "UPDATE 和 DELETE 语句必须携带合法的 WHERE 条件，严禁出现无条件的批量全量改动脚本。".into(),
                    pattern_hint: Some("(UPDATE|DELETE FROM)[^;]+;$".into()),
                },
            ],
            SupportedLanguage::Unknown => Vec::new(),
        }
    }

    /// 根据一组文件路径，自动汇总相关语言的全部审查规范并生成 Markdown 验收清单
    pub fn build_verification_prompt(paths: &[&Path]) -> String {
        let mut detected_languages = HashSet::new();
        for p in paths {
            let lang = LanguageSniffer::sniff_path(p);
            if lang != SupportedLanguage::Unknown {
                detected_languages.insert(lang);
            }
        }

        if detected_languages.is_empty() {
            return String::new();
        }

        let mut output = String::from("\n## 确定性语言规范与审查红线 (Verification Checklist)\n");
        output.push_str("基于本次变更涉及的代码语言特征，生成与自检必须严格遵循以下规范：\n\n");

        for lang in detected_languages {
            let rules = Self::get_rules_for_language(lang);
            if rules.is_empty() {
                continue;
            }
            output.push_str(&format!("### 语言规范：{}\n", lang.as_str().to_uppercase()));
            for rule in rules {
                output.push_str(&format!(
                    "- **[{}] {}**: {}\n",
                    rule.category.display_name(),
                    rule.title,
                    rule.description
                ));
            }
            output.push('\n');
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn sniffs_languages_correctly() {
        assert_eq!(LanguageSniffer::sniff_path(Path::new("src/main.rs")), SupportedLanguage::Rust);
        assert_eq!(LanguageSniffer::sniff_path(Path::new("App.tsx")), SupportedLanguage::TypeScript);
        assert_eq!(LanguageSniffer::sniff_path(Path::new("service.go")), SupportedLanguage::Go);
        assert_eq!(LanguageSniffer::sniff_path(Path::new("UserController.java")), SupportedLanguage::Java);
        assert_eq!(LanguageSniffer::sniff_path(Path::new("script.py")), SupportedLanguage::Python);
    }

    #[test]
    fn provides_rules_for_java_and_go() {
        let java_rules = BuiltinRuleset::get_rules_for_language(SupportedLanguage::Java);
        assert!(!java_rules.is_empty());
        assert!(java_rules.iter().any(|r| r.id == "JAVA_NPE_CHAIN"));

        let go_rules = BuiltinRuleset::get_rules_for_language(SupportedLanguage::Go);
        assert!(!go_rules.is_empty());
        assert!(go_rules.iter().any(|r| r.id == "GO_UNCHECKED_ERR"));
    }

    #[test]
    fn builds_verification_prompt_checklist() {
        let paths = vec![
            Path::new("UserService.java"),
            Path::new("gateway.go"),
        ];
        let prompt = BuiltinRuleset::build_verification_prompt(&paths);
        assert!(prompt.contains("JAVA"));
        assert!(prompt.contains("GO"));
        assert!(prompt.contains("NPE"));
        assert!(prompt.contains("未校验的 error 返回值"));
    }
}
