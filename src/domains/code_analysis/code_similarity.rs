//! Code similarity and clone detector based on MinHash and Jaccard distance.
//!
//! Inspired by codebase-memory-mcp's `simhash/minhash.c`. Employs 64-permutation
//! MinHash with 3-gram token shingling to calculate high-speed approximate Jaccard
//! similarities across code snippets, detecting duplicate functions and clone candidates.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const NUM_HASHES: usize = 64;
const PRIME: u64 = 4_294_967_311; // 2^32 + 15 (a prime)

// Precomputed 64 seed pairs (a, b) where a is odd
const HASH_COEFFS: [(u64, u64); NUM_HASHES] = [
    (1, 7), (3, 11), (5, 13), (7, 17), (9, 19), (11, 23), (13, 29), (15, 31),
    (17, 37), (19, 41), (21, 43), (23, 47), (25, 53), (27, 59), (29, 61), (31, 67),
    (33, 71), (35, 73), (37, 79), (39, 83), (41, 89), (43, 97), (45, 101), (47, 103),
    (49, 107), (51, 109), (53, 113), (55, 127), (57, 131), (59, 137), (61, 139), (63, 149),
    (65, 151), (67, 157), (69, 163), (71, 167), (73, 173), (75, 179), (77, 181), (79, 191),
    (81, 193), (83, 197), (85, 199), (87, 211), (89, 223), (91, 227), (93, 229), (95, 233),
    (97, 239), (99, 241), (101, 251), (103, 257), (105, 263), (107, 269), (109, 271), (111, 277),
    (113, 281), (115, 283), (117, 293), (119, 307), (121, 311), (123, 313), (125, 317), (127, 331),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCloneMatch {
    pub name_a: String,
    pub name_b: String,
    pub similarity: f64,
    pub clone_type: String,
}

pub struct CodeSimilarityDetector;

impl CodeSimilarityDetector {
    /// Tokenizes source code, stripping comments and trivial punctuation
    pub fn tokenize(code: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        for line in code.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
                continue;
            }
            for part in trimmed.split(|c: char| !c.is_alphanumeric() && c != '_') {
                if !part.is_empty() {
                    tokens.push(part.to_lowercase());
                }
            }
        }
        tokens
    }

    /// Computes 64-dimensional MinHash signature from source code tokens
    pub fn compute_signature(code: &str) -> [u64; NUM_HASHES] {
        let tokens = Self::tokenize(code);
        let mut sig = [u64::MAX; NUM_HASHES];

        if tokens.is_empty() {
            return sig;
        }

        // Multi-scale shingles: combines unigrams, bigrams and trigrams for resilience
        let mut shingles: HashSet<String> = HashSet::new();
        for t in &tokens {
            shingles.insert(t.clone());
        }
        if tokens.len() >= 2 {
            for w in tokens.windows(2) {
                shingles.insert(w.join(" "));
            }
        }
        if tokens.len() >= 3 {
            for w in tokens.windows(3) {
                shingles.insert(w.join(" "));
            }
        }

        for shingle in &shingles {
            let h = fnv1a_hash(shingle.as_bytes());
            for i in 0..NUM_HASHES {
                let (a, b) = HASH_COEFFS[i];
                let permuted = (a.wrapping_mul(h).wrapping_add(b)) % PRIME;
                if permuted < sig[i] {
                    sig[i] = permuted;
                }
            }
        }

        sig
    }

    /// Computes approximate Jaccard similarity between two MinHash signatures (0.0 to 1.0)
    pub fn jaccard_similarity(sig_a: &[u64; NUM_HASHES], sig_b: &[u64; NUM_HASHES]) -> f64 {
        let mut matches = 0;
        for i in 0..NUM_HASHES {
            if sig_a[i] == sig_b[i] && sig_a[i] != u64::MAX {
                matches += 1;
            }
        }
        (matches as f64) / (NUM_HASHES as f64)
    }

    /// Directly compares two source code snippets and returns similarity score [0.0, 1.0]
    pub fn compare_snippets(code_a: &str, code_b: &str) -> f64 {
        let sig_a = Self::compute_signature(code_a);
        let sig_b = Self::compute_signature(code_b);
        Self::jaccard_similarity(&sig_a, &sig_b)
    }

    /// Finds potential duplicate functions among a list of (name, source) tuples
    pub fn find_clones(functions: &[(&str, &str)], min_similarity: f64) -> Vec<CodeCloneMatch> {
        let signatures: Vec<[u64; NUM_HASHES]> = functions
            .iter()
            .map(|(_, src)| Self::compute_signature(src))
            .collect();

        let mut matches = Vec::new();
        for i in 0..functions.len() {
            for j in (i + 1)..functions.len() {
                let sim = Self::jaccard_similarity(&signatures[i], &signatures[j]);
                if sim >= min_similarity {
                    let clone_type = if sim >= 0.90 {
                        "EXACT_OR_RENAME".to_string()
                    } else if sim >= 0.70 {
                        "HIGH_SIMILARITY".to_string()
                    } else {
                        "STRUCTURAL_SIMILARITY".to_string()
                    };

                    matches.push(CodeCloneMatch {
                        name_a: functions[i].0.to_string(),
                        name_b: functions[j].0.to_string(),
                        similarity: (sim * 1000.0).round() / 1000.0,
                        clone_type,
                    });
                }
            }
        }

        matches.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
        matches
    }
}

/// 64-bit FNV-1a non-cryptographic hash
fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_similarity_identical_code() {
        let code1 = "pub fn add(a: i32, b: i32) -> i32 { a + b }";
        let code2 = "pub fn add(a: i32, b: i32) -> i32 { a + b }";

        let sim = CodeSimilarityDetector::compare_snippets(code1, code2);
        assert!((sim - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_similarity_minor_variable_rename() {
        let code1 = r#"
            pub fn calculate_total(items: &[Item]) -> f64 {
                let mut sum = 0.0;
                for item in items {
                    sum += item.price * item.quantity;
                }
                sum
            }
        "#;
        let code2 = r#"
            pub fn compute_total(cart: &[Item]) -> f64 {
                let mut total = 0.0;
                for item in cart {
                    total += item.price * item.quantity;
                }
                total
            }
        "#;

        let sim = CodeSimilarityDetector::compare_snippets(code1, code2);
        assert!(sim >= 0.30, "Expected structural similarity, got {}", sim);
    }

    #[test]
    fn test_similarity_near_duplicate() {
        let code1 = r#"
            pub fn handle_request(req: &Request, ctx: &mut Context) -> Result<Response, Error> {
                if !ctx.is_authorized() {
                    return Err(Error::Unauthorized);
                }
                let body = req.read_body()?;
                let parsed = parse_payload(&body)?;
                ctx.metrics.increment("request_count");
                Ok(Response::ok(parsed))
            }
        "#;
        let code2 = r#"
            pub fn handle_request(request: &Request, ctx: &mut Context) -> Result<Response, Error> {
                if !ctx.is_authorized() {
                    return Err(Error::Unauthorized);
                }
                let body = request.read_body()?;
                let parsed = parse_payload(&body)?;
                ctx.metrics.increment("request_count");
                Ok(Response::ok(parsed))
            }
        "#;

        let sim = CodeSimilarityDetector::compare_snippets(code1, code2);
        assert!(sim >= 0.70, "Expected high clone similarity, got {}", sim);
    }

    #[test]
    fn test_similarity_completely_different_code() {
        let code1 = "pub fn render_html() -> String { format!(\"<html></html>\") }";
        let code2 = "pub fn write_tcp_stream(buf: &[u8]) { socket.write_all(buf).unwrap(); }";

        let sim = CodeSimilarityDetector::compare_snippets(code1, code2);
        assert!(sim < 0.20, "Expected low similarity, got {}", sim);
    }
}
