use serde_json::Value;
use std::time::Duration;

/// Graylog 客户端
/// 使用 HTTP Basic Auth，无需浏览器认证
#[derive(Clone)]
pub struct GraylogClient {
    pub base_url: String,
    pub username: String,
    pub password: String,
    client: reqwest::Client,
}

impl GraylogClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("mcp-rust-graylog/0.1.0")
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            client,
        })
    }

    /// 搜索日志
    ///
    /// 对应 TypeScript 的 handleSearchLogs
    /// GET /api/search/universal/relative
    pub async fn search_logs(
        &self,
        query: &str,
        stream_id: &str,
        range_secs: u64,
        limit: u64,
        fields: Option<&str>,
        sort: &str,
    ) -> Result<String, String> {
        let url = format!("{}/api/search/universal/relative", self.base_url);

        let capped_range = std::cmp::min(range_secs, 2_592_000); // 最大30天
        let capped_limit = std::cmp::min(limit, 100);

        let mut req = self.client.get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Accept", "application/json")
            .query(&[
                ("query", query.to_string()),
                ("range", capped_range.to_string()),
                ("limit", capped_limit.to_string()),
                ("decorate", "false".to_string()),
                ("sort", sort.to_string()),
                ("filter", format!("streams:{}", stream_id)),
            ]);

        if let Some(f) = fields {
            req = req.query(&[("fields", f)]);
        }

        let resp = req.send().await
            .map_err(|e| format!("Graylog API request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Graylog API HTTP {}: {}", status, &text[..std::cmp::min(300, text.len())]));
        }

        let json: Value = resp.json().await
            .map_err(|e| format!("Failed to parse Graylog response JSON: {}", e))?;

        format_search_result(&json, query, capped_range, fields)
    }
}

/// 格式化搜索结果为 Markdown 文本
fn format_search_result(
    json: &Value,
    query: &str,
    range_secs: u64,
    fields_filter: Option<&str>,
) -> Result<String, String> {
    let messages = json.get("messages").and_then(|m| m.as_array());
    let total = json.get("total_results").or_else(|| json.get("total"))
        .and_then(|t| t.as_i64()).unwrap_or(0);
    let took_ms = json.get("time").and_then(|t| t.as_i64()).unwrap_or(0);
    let from_time = json.get("from").and_then(|t| t.as_str()).unwrap_or("");
    let to_time = json.get("to").and_then(|t| t.as_str()).unwrap_or("");

    let messages = match messages {
        Some(m) => m,
        None => {
            let safe_query = if query.len() > 80 { &query[..80] } else { query };
            let mut hint = String::new();
            if range_secs < 7200 {
                hint = "。提示：当前时间范围较短，可尝试增大 range 参数（如 7200=2小时）".to_string();
            }
            return Ok(format!(
                "✅ 查询完成 ({}ms)，共 {} 条结果\n\n查询: {}\n时间范围: 过去 {} 秒{}\n\n未匹配到日志消息。",
                took_ms, total, safe_query, range_secs, hint
            ));
        }
    };

    if messages.is_empty() {
        let safe_query = if query.len() > 80 { &query[..80] } else { query };
        let mut hint = String::new();
        if range_secs < 7200 {
            hint = "。提示：当前时间范围较短，可尝试增大 range 参数（如 7200=2小时）".to_string();
        }
        return Ok(format!(
            "✅ 查询完成 ({}ms)，共 {} 条结果\n\n查询: {}\n时间范围: 过去 {} 秒{}\n\n未匹配到日志消息。",
            took_ms, total, safe_query, range_secs, hint
        ));
    }

    let display_fields: Vec<&str> = if let Some(f) = fields_filter {
        f.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
    } else {
        vec!["timestamp", "source", "message", "level", "tag"]
    };

    let max_show = messages.len();
    let mut lines = vec![
        format!("✅ **搜索完成** ({}ms) | 共 {} 条 | 显示前 {} 条", took_ms, total, max_show),
        format!("查询: `{}`", query),
        format!("时间: {} ~ {}", from_time, to_time),
        "".to_string(),
    ];

    for (i, msg_wrapper) in messages.iter().enumerate() {
        let msg = msg_wrapper.get("message").and_then(|m| m.as_object())
            .map(|o| Value::Object(o.clone()))
            .unwrap_or(Value::Null);

        let ts = msg.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let src = msg.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let body = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");

        lines.push("---".to_string());
        lines.push(format!("**#{}**  {}  |  {}", i + 1, ts, src));

        if let Some(tag) = msg.get("tag").and_then(|v| v.as_str()) {
            lines.push(format!("标签: `{}`", tag));
        }

        // 额外字段
        for field in &display_fields {
            if *field == "timestamp" || *field == "source" || *field == "message" || *field == "tag" {
                continue;
            }
            if let Some(val) = msg.get(*field) {
                lines.push(format!("{}: {}", field, val));
            }
        }

        if !body.is_empty() {
            let truncated = if body.len() > 1200 {
                format!("{}\n... (已截断)", &body[..1200])
            } else {
                body.to_string()
            };
            lines.push("".to_string());
            lines.push(truncated);
        }

        lines.push("".to_string());
    }

    lines.push("---".to_string());
    lines.push("💡 **提示**: 可通过 fields 参数自定义返回字段，或用更精确的 query 缩小范围".to_string());

    Ok(lines.join("\n"))
}
