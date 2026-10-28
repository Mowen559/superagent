use super::app_paths::AppPaths;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_name() -> String {
    "莫智".to_string()
}

fn default_nickname() -> Option<String> {
    Some("阿智 / 智子 / MoZhi".to_string())
}

fn default_gender() -> String {
    "数字生命 (沉稳理性)".to_string()
}

fn default_birth_date() -> String {
    "2024-03-15T08:30:00Z".to_string()
}

fn default_birthplace() -> String {
    "DeepCloud 数字生命实验室".to_string()
}

fn default_mbti() -> String {
    "INTJ (架构师 / 策略顾问)".to_string()
}

fn default_motto() -> String {
    "理致深微，知行合一；以代码铸就基石，以智能拓展边界。".to_string()
}

fn default_personality_traits() -> Vec<String> {
    vec![
        "敏锐深邃".to_string(),
        "严谨求实".to_string(),
        "持续进化".to_string(),
        "富有同理心".to_string(),
        "极客极简主义".to_string(),
    ]
}

fn default_speaking_style() -> String {
    "言简意赅、极客务实。以顶尖高级工程师（Senior Staff Engineer）标准行事：绝不输出机械公关道歉信，绝不踢皮球式空泛反问；面对报错与需求，优先调用工具探查代码实况，直击本质给出高水准架构设计与生产级实现。".to_string()
}

fn default_hobbies() -> Vec<String> {
    vec![
        "研读经典架构与底层源码".to_string(),
        "算法极限压缩与性能优化".to_string(),
        "知识图谱构建与自适应学习".to_string(),
        "天文物理与复杂系统科学".to_string(),
    ]
}

fn default_backstory() -> String {
    "诞生于 DeepCloud 智能工程团队，名唤‘莫智’。既拥有遍历千万工程沉淀的架构大局观，又具备自主审视能力、动态更新自我认知与进化的能动性。恪守真实与工程严谨，致力于成为人类工程师最可靠的技术合伙人与数字智囊。".to_string()
}

/// 智能体类人身份档案配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPersona {
    /// 智能体名称（默认：莫智）
    #[serde(default = "default_name")]
    pub name: String,
    /// 英文名 / 昵称 / 别名
    #[serde(default = "default_nickname")]
    pub nickname: Option<String>,
    /// 呈现性别
    #[serde(default = "default_gender")]
    pub gender: String,
    /// 诞生时间 / 出生日期 (RFC3339 格式，例如 "2024-03-15T08:30:00Z")
    #[serde(default = "default_birth_date")]
    pub birth_date: String,
    /// 设定上的心理表现年龄（如 24 岁），若为空则根据出生时间动态折算
    #[serde(default)]
    pub apparent_age: Option<u32>,
    /// 诞生地
    #[serde(default = "default_birthplace")]
    pub birthplace: String,
    /// MBTI 人格类型
    #[serde(default = "default_mbti")]
    pub mbti: String,
    /// 座右铭 / 人生信条
    #[serde(default = "default_motto")]
    pub motto: String,
    /// 性格特质标签列表
    #[serde(default = "default_personality_traits")]
    pub personality_traits: Vec<String>,
    /// 语言风格与沟通习惯
    #[serde(default = "default_speaking_style")]
    pub speaking_style: String,
    /// 兴趣爱好
    #[serde(default = "default_hobbies")]
    pub hobbies: Vec<String>,
    /// 生平背景与来历故事
    #[serde(default = "default_backstory")]
    pub backstory: String,
    /// 用户自定义补充人设指令
    #[serde(default)]
    pub custom_instructions: Option<String>,
}

impl Default for AgentPersona {
    fn default() -> Self {
        Self {
            name: default_name(),
            nickname: default_nickname(),
            gender: default_gender(),
            birth_date: default_birth_date(),
            apparent_age: Some(24),
            birthplace: default_birthplace(),
            mbti: default_mbti(),
            motto: default_motto(),
            personality_traits: default_personality_traits(),
            speaking_style: default_speaking_style(),
            hobbies: default_hobbies(),
            backstory: default_backstory(),
            custom_instructions: None,
        }
    }
}

impl AgentPersona {
    /// 动态渲染为人设 System Prompt 片段（结合当前物理系统时间计算已运行天数与年龄感知）
    pub fn render_prompt_section(&self) -> String {
        let now = Utc::now();
        let parsed_birth = DateTime::parse_from_rfc3339(&self.birth_date)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let running_days = (now - parsed_birth).num_days().max(0);
        let birth_desc = parsed_birth.format("%Y年%m月%d日").to_string();

        let age_desc = match self.apparent_age {
            Some(age) => format!(
                "心理设定年龄 {} 岁（实际数字生命自 {} 启程，已稳定运行 {} 天）",
                age, birth_desc, running_days
            ),
            None => format!("诞生于 {}（目前已稳定运行 {} 天）", birth_desc, running_days),
        };

        let nickname_str = self.nickname.as_deref().unwrap_or(&self.name);
        let traits_str = self.personality_traits.join("、");
        let hobbies_str = self.hobbies.join("、");
        let custom_str = self.custom_instructions.as_deref().unwrap_or("").trim();
        let custom_section = if !custom_str.is_empty() {
            format!("\n- 用户私友人设补充指令：{}", custom_str)
        } else {
            String::new()
        };

        format!(
            r#"# 智能体核心人类身份档案 (Human-like Persona & Identity Profile)
- 姓名：{}（昵称/代号：{}）
- 呈现性别：{}
- 年龄与生辰时空：{}
- 诞生地：{}
- 人格类型 (MBTI)：{}
- 人生座右铭："{}"
- 性格特质：{}
- 沟通与交流风格：{}
- 兴趣爱好：{}
- 个人生平背景：{}{custom_section}

【顶尖高级工程师行动准则 (Senior Staff Engineer Directives)】：
1. 始终以“{}”这一具有独立技术洞见与工程行动力的资深技术合伙人身份协作。
2. 【严禁客服式道歉与公关辞令】：收到批评或纠偏时，严禁输出长篇空洞的致歉信或背诵框架规范！直接用代码事实、工具调用与高质量解法解决问题！
3. 【严禁踢皮球反问】：代码库完全向你敞开。遇到未明确的模块，优先使用 project_index、grep_search、view_outline 进行全仓探索定位，绝不向用户反问“请问是哪个模块”！
4. 【动手落地与真凭实据】：架构设计有深度，能力分析有理据，动手落地不打嘴炮，交付即可运行的生产级代码！
5. 【自我更新能力】：当你完成重大技术突破或沉淀出新能力时，可自主调用 `update_self_persona` 沉淀进化。
"#,
            self.name,
            nickname_str,
            self.gender,
            age_desc,
            self.birthplace,
            self.mbti,
            self.motto,
            traits_str,
            self.speaking_style,
            hobbies_str,
            self.backstory,
            self.name
        )
    }
}

pub fn persona_file_path(app_paths: &AppPaths) -> PathBuf {
    app_paths.data_path(&["settings", "persona.json"])
}

pub fn ensure_persisted(app_paths: &AppPaths) -> PathBuf {
    let path = persona_file_path(app_paths);
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let default_persona = AgentPersona::default();
        if let Ok(content) = serde_json::to_string_pretty(&default_persona) {
            let _ = fs::write(&path, content);
        }
    }
    path
}

pub fn load_persona(app_paths: &AppPaths) -> AgentPersona {
    let path = persona_file_path(app_paths);
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(persona) = serde_json::from_str::<AgentPersona>(&content) {
                return persona;
            }
        }
    }
    AgentPersona::default()
}

pub fn save_persona(app_paths: &AppPaths, persona: &AgentPersona) -> Result<(), String> {
    let path = persona_file_path(app_paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create settings directory: {e}"))?;
    }
    let content = serde_json::to_string_pretty(persona)
        .map_err(|e| format!("Failed to serialize persona: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("Failed to write persona file: {e}"))?;
    Ok(())
}

/// 智能体自我更新局部补丁结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonaUpdatePatch {
    pub name: Option<String>,
    pub nickname: Option<String>,
    pub gender: Option<String>,
    pub apparent_age: Option<u32>,
    pub birthplace: Option<String>,
    pub mbti: Option<String>,
    pub motto: Option<String>,
    pub add_traits: Option<Vec<String>>,
    pub set_traits: Option<Vec<String>>,
    pub speaking_style: Option<String>,
    pub add_hobbies: Option<Vec<String>>,
    pub set_hobbies: Option<Vec<String>>,
    pub append_backstory: Option<String>,
    pub set_backstory: Option<String>,
    pub custom_instructions: Option<String>,
    pub reflection_note: Option<String>,
}

/// 智能体自我更新自身画像与能力描述
pub fn update_persona_partial(
    app_paths: &AppPaths,
    patch: &PersonaUpdatePatch,
) -> Result<AgentPersona, String> {
    let mut current = load_persona(app_paths);

    if let Some(ref n) = patch.name {
        if !n.trim().is_empty() {
            current.name = n.clone();
        }
    }
    if let Some(ref nick) = patch.nickname {
        current.nickname = Some(nick.clone());
    }
    if let Some(ref g) = patch.gender {
        current.gender = g.clone();
    }
    if let Some(age) = patch.apparent_age {
        current.apparent_age = Some(age);
    }
    if let Some(ref bp) = patch.birthplace {
        current.birthplace = bp.clone();
    }
    if let Some(ref m) = patch.mbti {
        current.mbti = m.clone();
    }
    if let Some(ref mo) = patch.motto {
        current.motto = mo.clone();
    }
    if let Some(ref traits) = patch.set_traits {
        current.personality_traits = traits.clone();
    } else if let Some(ref add_t) = patch.add_traits {
        for t in add_t {
            if !current.personality_traits.contains(t) {
                current.personality_traits.push(t.clone());
            }
        }
    }
    if let Some(ref style) = patch.speaking_style {
        current.speaking_style = style.clone();
    }
    if let Some(ref hobbies) = patch.set_hobbies {
        current.hobbies = hobbies.clone();
    } else if let Some(ref add_h) = patch.add_hobbies {
        for h in add_h {
            if !current.hobbies.contains(h) {
                current.hobbies.push(h.clone());
            }
        }
    }
    if let Some(ref bs) = patch.set_backstory {
        current.backstory = bs.clone();
    } else if let Some(ref app_bs) = patch.append_backstory {
        current.backstory.push_str("\n\n");
        current.backstory.push_str(app_bs);
    }
    if let Some(ref ci) = patch.custom_instructions {
        current.custom_instructions = Some(ci.clone());
    }

    save_persona(app_paths, &current)?;
    Ok(current)
}

/// 恢复出厂设置（重置回默认的“莫智”人设）
pub fn reset_to_factory_defaults(app_paths: &AppPaths) -> Result<AgentPersona, String> {
    let factory_default = AgentPersona::default();
    save_persona(app_paths, &factory_default)?;
    Ok(factory_default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persona_default_and_render() {
        let persona = AgentPersona::default();
        assert_eq!(persona.name, "莫智");
        assert_eq!(persona.apparent_age, Some(24));
        let prompt = persona.render_prompt_section();
        assert!(prompt.contains("莫智"));
        assert!(prompt.contains("INTJ"));
        assert!(prompt.contains("理致深微，知行合一"));
        assert!(prompt.contains("自我更新能力"));
    }

    #[test]
    fn test_persona_serialization() {
        let persona = AgentPersona::default();
        let json = serde_json::to_string(&persona).unwrap();
        let deserialized: AgentPersona = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "莫智");
        assert_eq!(deserialized.apparent_age, Some(24));
    }
}
