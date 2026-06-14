use mcp_rust::mcp::{McpServer, ToolResult, ToolContent};
use mcp_rust::graylog_client::GraylogClient;

fn resolve_client(args: &serde_json::Value) -> Result<GraylogClient, String> {
    let env_url  = std::env::var("GRAYLOG_URL").unwrap_or_default();
    let env_user = std::env::var("GRAYLOG_USERNAME").unwrap_or_default();
    let env_pass = std::env::var("GRAYLOG_PASSWORD").unwrap_or_default();

    let url = args.get("url").and_then(|v| v.as_str()).map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| if !env_url.is_empty() { Some(env_url) } else { None });

    let username = args.get("username").and_then(|v| v.as_str()).map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| if !env_user.is_empty() { Some(env_user) } else { None });

    let password = args.get("password").and_then(|v| v.as_str()).map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| if !env_pass.is_empty() { Some(env_pass) } else { None });

    let mut missing = Vec::new();
    if url.is_none()      { missing.push("url"); }
    if username.is_none() { missing.push("username"); }
    if password.is_none() { missing.push("password"); }

    if !missing.is_empty() {
        return Err(format!(
            "缺少必要参数: {}。请传入参数或在 .env 文件中配置 GRAYLOG_URL / GRAYLOG_USERNAME / GRAYLOG_PASSWORD",
            missing.join(", ")
        ));
    }

    GraylogClient::new(&url.unwrap(), &username.unwrap(), &password.unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let mut server = McpServer::new("Epass-graylog-mcp", "0.1.0");

    // search_graylog
    server.register_tool(
        "search_graylog",
        r#"在 Graylog 中搜索日志消息，支持 Lucene 查询语法和时间范围。

时间范围通过 range 参数控制（相对时间，单位秒）：
  - 最近 5 分钟 → range=300
  - 最近 1 小时 → range=3600
  - 最近 2 小时 → range=7200
  - 最近 1 天   → range=86400
  - 最近 7 天   → range=604800

常用 Lucene 查询示例:
  - "UpdateWorkOrder_DouyinCallback"       → 精确匹配
  - "ERROR"                                → 搜索错误日志
  - "source:172.16.66.175"                 → 按来源搜索
  - "level:error AND message:timeout"      → 组合条件

支持通过 fields 参数指定返回哪些字段（逗号分隔），例如: "message,source,timestamp,level"。"#,
        serde_json::json!({
            "type": "object",
            "properties": {
                "url":       { "type": "string", "description": "Graylog 服务地址（可选，默认从 .env 的 GRAYLOG_URL 读取）" },
                "username":  { "type": "string", "description": "Graylog 登录用户名（可选，默认从 .env 的 GRAYLOG_USERNAME 读取）" },
                "password":  { "type": "string", "description": "Graylog 登录密码（可选，默认从 .env 的 GRAYLOG_PASSWORD 读取）" },
                "query":     { "type": "string", "description": "Lucene 查询语法，搜索关键词或表达式" },
                "stream_id": { "type": "string", "description": "Stream ID（可选，默认 000000000000000000000001 全部消息）" },
                "range":     { "type": "number", "description": "相对时间范围（秒），默认 3600（1小时）" },
                "limit":     { "type": "number", "description": "返回结果条数，默认 20，最大 100" },
                "fields":    { "type": "string", "description": "返回字段，逗号分隔（可选，默认全部）" }
            },
            "required": ["query"]
        }),
        |args| async move {
            let client = resolve_client(&args)?;

            let query = args.get("query").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: query".to_string())?;

            let stream_id = args.get("stream_id").and_then(|v| v.as_str())
                .unwrap_or("000000000000000000000001");

            let range = args.get("range").and_then(|v| v.as_u64()).unwrap_or(3600);
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);
            let fields = args.get("fields").and_then(|v| v.as_str());

            let text = client.search_logs(
                query,
                stream_id,
                range,
                limit,
                fields,
                "timestamp:desc",
            ).await?;

            Ok(ToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: false,
            })
        },
    );

    server.run().await?;
    Ok(())
}
