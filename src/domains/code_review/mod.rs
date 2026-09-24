pub mod types;
pub mod rules;
pub mod engine;

#[cfg(test)]
mod tests;

pub use engine::CodeReviewEngine;
pub use rules::RuleRegistry;
pub use types::{ReviewCategory, ReviewComment, ReviewReport, ReviewRule, ReviewSeverity, ReviewStats};
