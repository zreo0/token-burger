use std::collections::HashMap;
use std::path::Path;

use crate::types::{ModelPrice, PricingTable};

/// 缓存目录（dev/prod 隔离）
fn cache_dir() -> Option<std::path::PathBuf> {
    let base = dirs::data_local_dir()?.join("token-burger");
    #[cfg(debug_assertions)]
    {
        Some(base.join("dev").join("pricing"))
    }
    #[cfg(not(debug_assertions))]
    {
        Some(base.join("pricing"))
    }
}

/// 当天缓存文件路径
fn today_cache_file() -> Option<std::path::PathBuf> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    cache_dir().map(|d| d.join(format!("model_pricing_{}.json", today)))
}

/// 加载定价表：当天缓存 > 远程 > 历史缓存 > bundled > fallback
pub fn load_pricing_table(resources_dir: &Path) -> PricingTable {
    // 1. 尝试加载当天缓存
    if let Some(cached) = load_today_cache() {
        log::info!("使用当天定价缓存");
        return cached;
    }

    // 2. 尝试远程获取并缓存
    if let Some(remote) = fetch_remote_pricing() {
        save_today_cache(&remote);
        log::info!("已从远程更新定价表");
        return remote;
    }

    // 3. 尝试最近的历史缓存
    if let Some(recent) = load_latest_cache() {
        log::info!("使用历史定价缓存");
        return recent;
    }

    // 4. 尝试 bundled 文件
    let default_path = resources_dir.join("default_pricing.json");
    if let Ok(content) = std::fs::read_to_string(&default_path) {
        if let Ok(table) = serde_json::from_str::<PricingTable>(&content) {
            return table;
        }
    }

    // 5. 硬编码 fallback
    log::warn!("无法加载定价数据，使用内置默认值");
    fallback_pricing()
}

/// 从当天缓存文件加载
fn load_today_cache() -> Option<PricingTable> {
    let path = today_cache_file()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 加载最近的历史缓存（按文件名降序找最新的）
fn load_latest_cache() -> Option<PricingTable> {
    let dir = cache_dir()?;
    if !dir.exists() {
        return None;
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("model_pricing_")
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
    for entry in entries {
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            if let Ok(table) = serde_json::from_str::<PricingTable>(&content) {
                return Some(table);
            }
        }
    }
    None
}

/// 保存到当天缓存文件
fn save_today_cache(table: &PricingTable) {
    if let Some(dir) = cache_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Some(path) = today_cache_file() {
            if let Ok(json) = serde_json::to_string_pretty(table) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}

/// 从 LiteLLM 远程获取最新定价（10s 超时，blocking）
pub fn fetch_remote_pricing() -> Option<PricingTable> {
    let url = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        log::warn!("远程定价请求失败: {}", resp.status());
        return None;
    }
    let content = resp.text().ok()?;
    parse_litellm_pricing(&content)
}

/// 解析 LiteLLM 格式的定价 JSON（忽略无价格字段的条目）
fn parse_litellm_pricing(content: &str) -> Option<PricingTable> {
    let raw: HashMap<String, serde_json::Value> = serde_json::from_str(content).ok()?;
    let mut table = PricingTable::new();
    for (model, value) in raw {
        if let Ok(price) = serde_json::from_value::<ModelPrice>(value) {
            table.insert(model, price);
        }
    }
    if table.is_empty() {
        None
    } else {
        Some(table)
    }
}

/// 计算费用
#[allow(dead_code)]
pub fn calculate_cost(
    model_id: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_create_tokens: i64,
    pricing: &PricingTable,
) -> f64 {
    let price = match pricing.get(model_id) {
        Some(p) => p,
        None => return 0.0,
    };

    let input_cost = input_tokens as f64 * price.input_cost_per_token;
    let output_cost = output_tokens as f64 * price.output_cost_per_token;
    let cache_read_cost = cache_read_tokens as f64 * price.cache_read_input_token_cost;
    let cache_create_cost = cache_create_tokens as f64 * price.cache_creation_input_token_cost;

    input_cost + output_cost + cache_read_cost + cache_create_cost
}

fn fallback_pricing() -> PricingTable {
    let mut table = HashMap::new();
    table.insert(
        "claude-sonnet-4-20250514".to_string(),
        ModelPrice {
            input_cost_per_token: 3.0 / 1_000_000.0,
            output_cost_per_token: 15.0 / 1_000_000.0,
            cache_read_input_token_cost: 0.30 / 1_000_000.0,
            cache_creation_input_token_cost: 3.75 / 1_000_000.0,
        },
    );
    table.insert(
        "claude-3-7-sonnet-20250219".to_string(),
        ModelPrice {
            input_cost_per_token: 3.0 / 1_000_000.0,
            output_cost_per_token: 15.0 / 1_000_000.0,
            cache_read_input_token_cost: 0.30 / 1_000_000.0,
            cache_creation_input_token_cost: 3.75 / 1_000_000.0,
        },
    );
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_cost() {
        let mut pricing = HashMap::new();
        pricing.insert(
            "test-model".to_string(),
            ModelPrice {
                input_cost_per_token: 3.0 / 1_000_000.0,
                output_cost_per_token: 15.0 / 1_000_000.0,
                cache_read_input_token_cost: 0.30 / 1_000_000.0,
                cache_creation_input_token_cost: 3.75 / 1_000_000.0,
            },
        );

        let cost = calculate_cost("test-model", 1_000_000, 100_000, 0, 0, &pricing);
        // 3.0 + 1.5 = 4.5
        assert!((cost - 4.5).abs() < 0.001);
    }

    #[test]
    fn test_calculate_cost_unknown_model() {
        let pricing = HashMap::new();
        let cost = calculate_cost("unknown", 1000, 500, 0, 0, &pricing);
        assert!((cost - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_fallback_pricing() {
        let table = fallback_pricing();
        assert!(table.contains_key("claude-sonnet-4-20250514"));
    }
}
