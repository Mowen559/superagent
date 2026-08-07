use std::collections::HashMap;

/// Module aliases for matching project context
pub fn module_aliases() -> HashMap<&'static str, Vec<&'static str>> {
    let mut map = HashMap::new();
    map.insert("order", vec!["order", "orders"]);
    map.insert("payment", vec!["payment", "payments", "pay"]);
    map.insert("coupon", vec!["coupon", "discount"]);
    map.insert("inventory", vec!["inventory", "stock"]);
    map.insert("auth", vec!["auth", "login", "permission"]);
    map.insert("user", vec!["user", "account"]);
    map.insert("memory", vec!["memory", "supermemory"]);
    map.insert("reverse-engineering", vec!["reverse", "engineering", "blueprint"]);
    map
}

pub fn default_business_flow() -> Vec<&'static str> {
    vec![
        "Receive requirement",
        "Extract intent and business objects",
        "Enrich with project context",
        "Generate executable development tasks",
    ]
}
