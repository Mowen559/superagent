//! Architectural Linter & Self-Healing Guardrail for DeepCloud
//! Scans modified files in real-time against core project ironclads defined in AGENTS.md.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinterAdvisory {
    pub rule_name: &'static str,
    pub violation: &'static str,
    pub prescribed_fix: &'static str,
}

pub struct ArchitecturalLinter;

impl ArchitecturalLinter {
    pub fn lint(path: &Path, content: &str) -> Vec<LinterAdvisory> {
        let mut advisories = Vec::new();
        let path_str = path.to_string_lossy().replace('\\', "/");

        // 1. Java-specific rules
        if path_str.ends_with(".java") {
            Self::lint_java(&path_str, content, &mut advisories);
        }

        // 2. Frontend-specific rules (TypeScript/TSX/JSX)
        if path_str.ends_with(".tsx") || path_str.ends_with(".ts") || path_str.ends_with(".jsx") {
            Self::lint_frontend(&path_str, content, &mut advisories);
        }

        advisories
    }

    fn lint_java(path_str: &str, content: &str, advisories: &mut Vec<LinterAdvisory>) {
        let is_controller = path_str.ends_with("Controller.java");

        // Rule 1: Controller must return Result<T> and not use raw maps with "code", 200
        if is_controller {
            if (content.contains("\"code\", 200") || content.contains("\"code\", 0"))
                && !content.contains("Result.ok(")
            {
                advisories.push(LinterAdvisory {
                    rule_name: "Controller Response Contract",
                    violation: "Found manual map put with numeric code 200/0. Frontend strictly expects String code and Result<T> format.",
                    prescribed_fix: "Inherit BaseController and return Result<T> via Result.ok(data). Do not handcraft response Maps.",
                });
            }

            // Rule 2: Controller method should have @RequirePermission
            if (content.contains("@PostMapping") || content.contains("@GetMapping") || content.contains("@PutMapping") || content.contains("@DeleteMapping"))
                && !content.contains("@RequirePermission")
                && !path_str.contains("/auth/")
                && !path_str.contains("/internal/")
            {
                advisories.push(LinterAdvisory {
                    rule_name: "Permission Annotation Required",
                    violation: "Controller endpoint lacks @RequirePermission annotation.",
                    prescribed_fix: "Annotate endpoint with @RequirePermission(\"module:resource:action\") to guarantee security enforcement.",
                });
            }
        }

        // Rule 3: JPA / Hibernate strictly prohibited (MyBatis-Plus only)
        if content.contains("extends JpaRepository")
            || content.contains("org.springframework.data.jpa")
            || content.contains("javax.persistence")
            || content.contains("jakarta.persistence.Entity")
        {
            advisories.push(LinterAdvisory {
                rule_name: "Forbidden ORM Technology (JPA)",
                violation: "Detected JPA/Hibernate imports or annotations. Project mandates MyBatis-Plus 3.5 exclusively.",
                prescribed_fix: "Use MyBatis-Plus BaseMapper<T> with LambdaQueryWrapper instead of JPA.",
            });
        }

        // Rule 4: MyBatis-Plus hardcoded column strings prohibited
        if content.contains(".eq(\"") || content.contains(".like(\"") || content.contains(".orderByDesc(\"") {
            advisories.push(LinterAdvisory {
                rule_name: "MyBatis-Plus Type Safety",
                violation: "Detected hardcoded column name strings in QueryWrapper (e.g. .eq(\"status\", ...)).",
                prescribed_fix: "Use LambdaQueryWrapper with method references (e.g. .eq(EntityDO::getStatus, ...)) to prevent schema drift bugs.",
            });
        }

        // Rule 5: Catching BusinessException inside Controller or Service
        if content.contains("catch (BusinessException") {
            advisories.push(LinterAdvisory {
                rule_name: "Business Exception Handling",
                violation: "Detected catch block swallowing BusinessException.",
                prescribed_fix: "Do not catch BusinessException in Controller or Service. Let it propagate to the global @RestControllerAdvice.",
            });
        }

        // Rule 6: Plaintext credentials prohibited
        if (content.contains("password = \"") || content.contains("secret = \"") || content.contains("apikey = \""))
            && !content.contains("vault")
            && !content.contains("secretRef")
            && !path_str.contains("/test/")
        {
            advisories.push(LinterAdvisory {
                rule_name: "Vault Credential Storage",
                violation: "Possible hardcoded credential string detected.",
                prescribed_fix: "Store secrets in HashiCorp Vault and reference them via secretRef.",
            });
        }
    }

    fn lint_frontend(path_str: &str, content: &str, advisories: &mut Vec<LinterAdvisory>) {
        // Skip test files
        if path_str.contains("__tests__") || path_str.contains(".test.") || path_str.contains(".spec.") {
            return;
        }

        // Rule 1: Manual checking of response code in pages
        if content.contains("res.code === \"200\"") || content.contains("res.code === 200") || content.contains("data.code === 200") {
            advisories.push(LinterAdvisory {
                rule_name: "Frontend API Unwrapping",
                violation: "Found manual if (res.code === \"200\") check.",
                prescribed_fix: "Remove manual code check. The axios interceptor in lib/api.ts already unwraps response bodies and throws on non-200.",
            });
        }

        // Rule 2: Hardcoded fixed Tailwind colors in UI components
        if (path_str.ends_with(".tsx") || path_str.ends_with(".jsx"))
            && (content.contains("bg-amber-") || content.contains("bg-blue-600") || content.contains("text-red-500") || content.contains("bg-gray-100"))
        {
            advisories.push(LinterAdvisory {
                rule_name: "Semantic Tailwind Colors",
                violation: "Detected hardcoded Tailwind fixed color classes (e.g. bg-amber-*, bg-blue-*, text-red-*).",
                prescribed_fix: "Use semantic Tailwind classes mapped to CSS variables (e.g. bg-muted, text-muted-foreground, bg-primary, text-destructive) to support theming.",
            });
        }
    }

    pub fn format_advisories(advisories: &[LinterAdvisory]) -> Option<String> {
        if advisories.is_empty() {
            return None;
        }

        let mut out = String::from("\n⚠️ [ARCHITECTURAL LINTER ADVISORY - CONTRACT INTEGRITY CHECK]\nPotential deviations from DeepCloud AGENTS.md detected in this edit:");
        for adv in advisories {
            out.push_str(&format!(
                "\n- [{}] {}\n  Fix: {}",
                adv.rule_name, adv.violation, adv.prescribed_fix
            ));
        }
        out.push_str("\nPlease review. If this was unintended, self-correct in your next step.");
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_lint_controller_missing_permission_and_map_response() {
        let code = r#"
            @RestController
            @RequestMapping("/api/v1/jobs")
            public class JobController {
                @PostMapping
                public Map<String, Object> createJob() {
                    Map<String, Object> map = new HashMap<>();
                    map.put("code", 200);
                    return map;
                }
            }
        "#;
        let advisories = ArchitecturalLinter::lint(&PathBuf::from("src/api/JobController.java"), code);
        assert_eq!(advisories.len(), 2);
        assert_eq!(advisories[0].rule_name, "Controller Response Contract");
        assert_eq!(advisories[1].rule_name, "Permission Annotation Required");
    }

    #[test]
    fn test_lint_jpa_prohibition() {
        let code = r#"
            import org.springframework.data.jpa.repository.JpaRepository;
            public interface UserRepository extends JpaRepository<User, Long> {}
        "#;
        let advisories = ArchitecturalLinter::lint(&PathBuf::from("src/domain/UserRepository.java"), code);
        assert_eq!(advisories.len(), 1);
        assert_eq!(advisories[0].rule_name, "Forbidden ORM Technology (JPA)");
    }

    #[test]
    fn test_lint_frontend_semantic_colors_and_unwrapping() {
        let code = r#"
            export function UserBadge() {
                if (res.code === "200") {
                    return <div className="bg-amber-600">Active</div>;
                }
            }
        "#;
        let advisories = ArchitecturalLinter::lint(&PathBuf::from("src/components/UserBadge.tsx"), code);
        assert_eq!(advisories.len(), 2);
        assert_eq!(advisories[0].rule_name, "Frontend API Unwrapping");
        assert_eq!(advisories[1].rule_name, "Semantic Tailwind Colors");
    }
}
