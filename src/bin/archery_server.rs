use mcp_rust::mcp::{McpServer, ToolResult, ToolContent};
use mcp_rust::archery_client::ArcheryClient;
use serde_json::Value;

fn resolve_client(args: &Value) -> Result<ArcheryClient, String> {
    let env_url = std::env::var("ARCHERY_URL").unwrap_or_default();
    let env_user = std::env::var("ARCHERY_USERNAME").unwrap_or_default();
    let env_pass = std::env::var("ARCHERY_PASSWORD").unwrap_or_default();

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
    if url.is_none() { missing.push("url"); }
    if username.is_none() { missing.push("username"); }
    if password.is_none() { missing.push("password"); }

    if !missing.is_empty() {
        return Err(format!(
            "缺少必要参数: {}。请传入参数或在 .env 文件中配置 ARCHERY_URL / ARCHERY_USERNAME / ARCHERY_PASSWORD",
            missing.join(", ")
        ));
    }

    let login_timeout = args.get("loginTimeout").and_then(|v| v.as_u64()).unwrap_or(15000);

    Ok(ArcheryClient::new(
        &url.unwrap(),
        &username.unwrap(),
        &password.unwrap(),
        login_timeout,
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env if present
    dotenvy::dotenv().ok();

    let mut server = McpServer::new("Epass-archery-mcp", "0.1.0");

    // 1. list_archery_instances
    server.register_tool(
        "list_archery_instances",
        "列出当前用户在 Archery 上有权限访问的所有数据库实例。\n\n首次使用 Archery 时应先调用此工具查看可用的数据库实例。\n酒店类数据库实例以 \"hotel_distribution\" 开头（同程FP/同程SP/抖音SP/公共）。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Archery 服务地址（可选，默认从 .env 读取）" },
                "username": { "type": "string", "description": "Archery 登录用户名（可选，默认从 .env 读取）" },
                "password": { "type": "string", "description": "Archery 登录密码（可选，默认从 .env 读取）" },
                "loginTimeout": { "type": "number", "description": "登录超时(ms)，默认 15000" }
            },
            "required": []
        }),
        |args| async move {
            let client = resolve_client(&args)?;
            let text = client.list_instances().await?;
            Ok(ToolResult {
                content: vec![ToolContent::Text { text: text.to_string() }],
                is_error: false,
            })
        }
    );

    // 2. list_archery_databases
    server.register_tool(
        "list_archery_databases",
        "列出指定数据库实例下的所有数据库。\n\n需先调用 list_archery_instances 获取 instance 名称。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Archery 服务地址" },
                "username": { "type": "string", "description": "Archery 登录用户名" },
                "password": { "type": "string", "description": "Archery 登录密码（可选，默认从 .env 读取）" },
                "instance": { "type": "string", "description": "数据库实例名称" },
                "loginTimeout": { "type": "number", "description": "登录超时(ms)，默认 15000" }
            },
            "required": ["instance"]
        }),
        |args| async move {
            let client = resolve_client(&args)?;
            let instance = args.get("instance").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: instance".to_string())?;
            let text = client.list_databases(instance).await?;
            Ok(ToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: false,
            })
        }
    );

    // 3. list_archery_tables
    server.register_tool(
        "list_archery_tables",
        "列出指定数据库下的所有数据表。\n\n需先调用 list_archery_databases 获取 database 名称。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Archery 服务地址" },
                "username": { "type": "string", "description": "Archery 登录用户名" },
                "password": { "type": "string", "description": "Archery 登录密码（可选，默认从 .env 读取）" },
                "instance": { "type": "string", "description": "数据库实例名称" },
                "database": { "type": "string", "description": "数据库名称" },
                "loginTimeout": { "type": "number", "description": "登录超时(ms)，默认 15000" }
            },
            "required": ["instance", "database"]
        }),
        |args| async move {
            let client = resolve_client(&args)?;
            let instance = args.get("instance").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: instance".to_string())?;
            let database = args.get("database").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: database".to_string())?;
            let text = client.list_tables(instance, database).await?;
            Ok(ToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: false,
            })
        }
    );

    // 4. describe_archery_table
    server.register_tool(
        "describe_archery_table",
        "查看指定表的字段结构、类型、注释等信息（通过 SHOW CREATE TABLE 获取 DDL）。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Archery 服务地址" },
                "username": { "type": "string", "description": "Archery 登录用户名" },
                "password": { "type": "string", "description": "Archery 登录密码（可选，默认从 .env 读取）" },
                "instance": { "type": "string", "description": "数据库实例名称" },
                "database": { "type": "string", "description": "数据库名称" },
                "table": { "type": "string", "description": "表名" },
                "loginTimeout": { "type": "number", "description": "登录超时(ms)，默认 15000" }
            },
            "required": ["instance", "database", "table"]
        }),
        |args| async move {
            let client = resolve_client(&args)?;
            let instance = args.get("instance").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: instance".to_string())?;
            let database = args.get("database").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: database".to_string())?;
            let table = args.get("table").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: table".to_string())?;
            let text = client.describe_table(instance, database, table).await?;
            Ok(ToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: false,
            })
        }
    );

    // 5. execute_archery_sql
    server.register_tool(
        "execute_archery_sql",
        "在指定的数据库实例上执行 SQL 查询，返回结构化结果（Markdown 表格）。\n\n支持 SELECT、SHOW、DESCRIBE 等查询语句。\n酒店类数据库实例以 hotel_distribution 开头（同程FP/同程SP/抖音SP/公共）。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Archery 服务地址" },
                "username": { "type": "string", "description": "Archery 登录用户名" },
                "password": { "type": "string", "description": "Archery 登录密码（可选，默认从 .env 读取）" },
                "instance": { "type": "string", "description": "数据库实例名称" },
                "database": { "type": "string", "description": "数据库名称（可选，默认使用实例同名数据库）" },
                "sql": { "type": "string", "description": "SQL 查询语句" },
                "limit": { "type": "number", "description": "结果条数限制，默认 100" },
                "loginTimeout": { "type": "number", "description": "登录超时(ms)，默认 15000" },
                "queryTimeout": { "type": "number", "description": "查询超时(ms)，默认 60000" }
            },
            "required": ["instance", "sql"]
        }),
        |args| async move {
            let client = resolve_client(&args)?;
            let instance = args.get("instance").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: instance".to_string())?;
            let sql = args.get("sql").and_then(|v| v.as_str())
                .ok_or_else(|| "缺少必要参数: sql".to_string())?;
            let database = args.get("database").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100);
            let query_timeout = args.get("queryTimeout").and_then(|v| v.as_u64()).unwrap_or(60000);
            
            let text = client.execute_sql(instance, database, sql, limit, query_timeout).await?;
            Ok(ToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: false,
            })
        }
    );

    server.run().await?;
    Ok(())
}
