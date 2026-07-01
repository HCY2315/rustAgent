use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use regex::Regex;
use reqwest::cookie::CookieStore;

#[derive(Clone)]
pub struct ArcherySession {
    pub base_url: String,
    pub client: reqwest::Client,
    pub jar: Arc<reqwest::cookie::Jar>,
    pub csrf_token: String,
}

static SESSION_CACHE: OnceLock<Mutex<HashMap<(String, String), (ArcherySession, Instant)>>> = OnceLock::new();

fn get_cached_session(url: &str, username: &str) -> Option<ArcherySession> {
    let cache_lock = SESSION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache = cache_lock.lock().unwrap();
    if let Some((session, expires_at)) = cache.get(&(url.to_string(), username.to_string())) {
        if Instant::now() < *expires_at {
            return Some(session.clone());
        }
    }
    None
}

fn set_cached_session(url: &str, username: &str, session: ArcherySession) {
    let cache_lock = SESSION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache_lock.lock().unwrap();
    let expires_at = Instant::now() + Duration::from_secs(30 * 60);
    cache.insert((url.to_string(), username.to_string()), (session, expires_at));
}

#[derive(Clone)]
pub struct ArcheryClient {
    pub url: String,
    pub username: String,
    pub password: String,
    pub login_timeout_ms: u64,
}

impl ArcheryClient {
    pub fn new(url: &str, username: &str, password: &str, login_timeout_ms: u64) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            login_timeout_ms,
        }
    }

    pub async fn get_session(&self) -> Result<ArcherySession, String> {
        if let Some(session) = get_cached_session(&self.url, &self.username) {
            let test_url = format!("{}/group/user_all_instances/?tag_codes%5B%5D=can_read", session.base_url);
            let test_res = session.client.get(&test_url)
                .header("Accept", "application/json")
                .header("X-CSRFToken", &session.csrf_token)
                .header("X-Requested-With", "XMLHttpRequest")
                .header("Referer", format!("{}/sqlquery/", session.base_url))
                .send()
                .await;

            match test_res {
                Ok(res) if res.status().is_success() => {
                    eprintln!("[Archery] Using cached session");
                    return Ok(session);
                }
                _ => {
                    eprintln!("[Archery] Cached session validation failed, re-authenticating");
                }
            }
        }

        let session = login(&self.url, &self.username, &self.password, self.login_timeout_ms).await?;
        set_cached_session(&self.url, &self.username, session.clone());
        Ok(session)
    }

    pub async fn list_instances(&self) -> Result<Value, String> {
        let session = self.get_session().await?;
        let url = format!("{}/group/user_all_instances/?tag_codes%5B%5D=can_read", session.base_url);

        let resp = session.client.get(&url)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("X-CSRFToken", &session.csrf_token)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", format!("{}/sqlquery/", session.base_url))
            .send()
            .await
            .map_err(|e| format!("List instances request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("List instances returned HTTP {}", resp.status()));
        }

        let json: Value = resp.json().await
            .map_err(|e| format!("Failed to parse list instances JSON: {}", e))?;

        let instances = json.get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| "Missing 'data' array in response".to_string())?;

        if instances.is_empty() {
            return Ok(json!({
                "message": "当前用户没有可访问的数据库实例。",
                "instances": []
            }));
        }

        let mut hotel = Vec::new();
        let mut other = Vec::new();

        for inst in instances {
            let name = inst.get("instance_name")
                .or_else(|| inst.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| inst.to_string());

            if name.starts_with("hotel_distribution") {
                hotel.push(name);
            } else {
                other.push(name);
            }
        }

        let result = json!({
            "message": format!("登录成功，共 {} 个可用数据库实例", instances.len()),
            "hotel": hotel,
            "other": other,
            "instances": instances
        });

        Ok(result)
    }

    pub async fn list_databases(&self, instance: &str) -> Result<String, String> {
        let session = self.get_session().await?;
        let url = format!(
            "{}/instance/instance_resource/?instance_name={}&resource_type=database",
            session.base_url,
            urlencoding::encode(instance)
        );

        let resp = session.client.get(&url)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("X-CSRFToken", &session.csrf_token)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", format!("{}/sqlquery/", session.base_url))
            .send()
            .await
            .map_err(|e| format!("List databases request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("List databases returned HTTP {}", resp.status()));
        }

        let json: Value = resp.json().await
            .map_err(|e| format!("Failed to parse list databases JSON: {}", e))?;

        let databases = json.get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| format!("实例 \"{}\" 下没有数据库。", instance))?;

        if databases.is_empty() {
            return Ok(format!("实例 \"{}\" 下没有数据库。", instance));
        }

        let mut lines = vec![
            format!("📂 **实例 \"{}\" 下的数据库 ({}个)**", instance, databases.len()),
            "".to_string(),
        ];

        for db in databases {
            if let Some(db_name) = db.as_str() {
                lines.push(format!("   - {}", db_name));
            }
        }

        lines.push("".to_string());
        lines.push("---".to_string());
        lines.push("💡 **下一步**: 选择数据库后可使用:".to_string());
        lines.push("   1. list_archery_tables → 查看表列表".to_string());
        lines.push("   2. execute_archery_sql → 直接执行 SQL".to_string());

        Ok(lines.join("\n"))
    }

    pub async fn list_tables(&self, instance: &str, database: &str) -> Result<String, String> {
        let session = self.get_session().await?;
        let url = format!(
            "{}/instance/instance_resource/?instance_name={}&db_name={}&resource_type=table",
            session.base_url,
            urlencoding::encode(instance),
            urlencoding::encode(database)
        );

        let resp = session.client.get(&url)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("X-CSRFToken", &session.csrf_token)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", format!("{}/sqlquery/", session.base_url))
            .send()
            .await
            .map_err(|e| format!("List tables request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("List tables returned HTTP {}", resp.status()));
        }

        let json: Value = resp.json().await
            .map_err(|e| format!("Failed to parse list tables JSON: {}", e))?;

        let tables = json.get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| format!("数据库 \"{}\" 中没有表。", database))?;

        if tables.is_empty() {
            return Ok(format!("数据库 \"{}\" 中没有表。", database));
        }

        let mut lines = vec![
            format!("📋 **数据库 \"{}\" 中的表 ({}个)**", database, tables.len()),
            "".to_string(),
        ];

        for t in tables {
            if let Some(table_name) = t.as_str() {
                lines.push(format!("   - {}", table_name));
            }
        }

        lines.push("".to_string());
        lines.push("---".to_string());
        lines.push("💡 **下一步**: 选择表后可使用:".to_string());
        lines.push("   1. describe_archery_table → 查看表结构".to_string());
        lines.push("   2. execute_archery_sql → 执行 SQL 查询".to_string());
        Ok(lines.join("\n"))
    }

    pub async fn describe_table(&self, instance: &str, database: &str, table: &str) -> Result<String, String> {
        let session = self.get_session().await?;
        
        let params = [
            ("instance_name", instance),
            ("db_name", database),
            ("schema_name", ""),
            ("tb_name", table),
        ];

        let resp = session.client.post(format!("{}/instance/describetable/", session.base_url))
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
            .header("X-CSRFToken", &session.csrf_token)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", format!("{}/sqlquery/", session.base_url))
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Describe table request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Describe table returned HTTP {}", resp.status()));
        }

        let json: Value = resp.json().await
            .map_err(|e| format!("Failed to parse describe table JSON: {}", e))?;

        let status = json.get("status").and_then(|s| s.as_i64()).unwrap_or(-1);
        if status != 0 {
            let error_msg = json.get("msg")
                .or_else(|| json.get("error"))
                .and_then(|e| e.as_str())
                .unwrap_or("未知错误");
            return Err(format!("获取表结构失败: {}", error_msg));
        }

        let rows = json.get("data")
            .and_then(|d| d.get("rows"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| format!("表 \"{}\" 没有返回结构信息。", table))?;

        if rows.is_empty() {
            return Ok(format!("表 \"{}\" 没有返回结构信息。", table));
        }

        let mut lines = vec![
            format!("## 📐 表结构: {}", table),
            "".to_string(),
            format!("**实例**: {}", instance),
            format!("**数据库**: {}", database),
            "".to_string(),
        ];

        // Append DDL
        let mut ddl_str = String::new();
        for r in rows {
            if let Some(arr) = r.as_array() {
                if arr.len() >= 2 {
                    if let Some(ddl) = arr[1].as_str() {
                        ddl_str = ddl.to_string();
                        lines.push("```sql".to_string());
                        lines.push(ddl_str.clone());
                        lines.push("```".to_string());
                        break;
                    }
                }
            }
        }

        // Summary Fields
        if !ddl_str.is_empty() {
            let re = field_regex();
            let mut fields = Vec::new();

            for line in ddl_str.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('`') 
                    && !trimmed.starts_with("PRIMARY KEY") 
                    && !trimmed.starts_with("KEY") 
                    && !trimmed.starts_with("UNIQUE KEY") 
                    && !trimmed.starts_with("CONSTRAINT") 
                {
                    if let Some(caps) = re.captures(trimmed) {
                        let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                        let type_str = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                        let comment = caps.get(3).map(|m| m.as_str()).unwrap_or("");
                        fields.push((name, type_str, comment));
                    }
                }
            }

            if !fields.is_empty() {
                lines.push("".to_string());
                lines.push(format!("**字段摘要 ({}个)**", fields.len()));
                lines.push("".to_string());
                lines.push("| 字段名 | 类型 | 注释 |".to_string());
                lines.push("| --- | --- | --- |".to_string());
                for (name, type_str, comment) in fields {
                    lines.push(format!("| `{}` | {} | {} |", name, type_str.trim(), comment));
                }
            }
        }

        lines.push("".to_string());
        lines.push("---".to_string());
        lines.push("💡 **下一步**: 使用 execute_archery_sql 执行查询".to_string());

        Ok(lines.join("\n"))
    }

    pub async fn execute_sql(
        &self,
        instance: &str,
        database: Option<&str>,
        sql: &str,
        limit: u64,
        query_timeout_ms: u64,
    ) -> Result<String, String> {
        let session = self.get_session().await?;
        
        let db_name = database.unwrap_or(instance);
        let params = [
            ("instance_name", instance.to_string()),
            ("db_name", db_name.to_string()),
            ("sql_content", sql.to_string()),
            ("limit_num", limit.to_string()),
        ];

        let start_time = Instant::now();
        let resp = session.client.post(format!("{}/query/", session.base_url))
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
            .header("X-CSRFToken", &session.csrf_token)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", format!("{}/sqlquery/", session.base_url))
            .form(&params)
            .timeout(Duration::from_millis(query_timeout_ms))
            .send()
            .await
            .map_err(|e| format!("SQL Query request failed: {}", e))?;

        let elapsed = start_time.elapsed().as_millis();

        if !resp.status().is_success() {
            return Err(format!("SQL Query returned HTTP {}", resp.status()));
        }

        let content_type = resp.headers()
            .get("content-type")
            .and_then(|c| c.to_str().ok())
            .unwrap_or("")
            .to_string();

        let text = resp.text().await
            .map_err(|e| format!("Failed to read response text: {}", e))?;

        if !content_type.contains("json") {
            let preview = if text.len() > 2000 {
                format!("{}\n... (已截断)", &text[..2000])
            } else {
                text
            };
            return Ok(format!("✅ 查询完成 ({}ms)\n\n{}", elapsed, preview));
        }

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Failed to parse query response JSON: {}", e))?;

        let status = json.get("status").and_then(|s| s.as_i64());
        if status == Some(1) || status == Some(2) || json.get("error").is_some() || json.get("err").is_some() {
            let err_msg = json.get("error")
                .or_else(|| json.get("err"))
                .or_else(|| json.get("msg"))
                .and_then(|e| e.as_str())
                .unwrap_or("未知错误");
            return Err(format!("⚠️ SQL 执行错误: {}", err_msg));
        }

        let raw_data = json.get("data")
            .or_else(|| json.get("results"))
            .unwrap_or(&json);

        let rows_data = raw_data.get("rows").unwrap_or(raw_data);
        
        let column_names_val = raw_data.get("column_list")
            .or_else(|| raw_data.get("column_names"))
            .or_else(|| json.get("column_names"))
            .or_else(|| json.get("columns"))
            .or_else(|| json.get("headers"));

        let data = rows_data.as_array().map(|v| v.as_slice()).unwrap_or(&[]);
        if data.is_empty() {
            let msg = json.get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("查询成功，未返回数据（0 条记录）。");
            return Ok(format!("✅ {}", msg));
        }

        let mut headers: Vec<String> = Vec::new();
        if let Some(cols) = column_names_val.and_then(|c| c.as_array()) {
            if !cols.is_empty() {
                headers = cols.iter().map(|c| {
                    if c.is_string() {
                        c.as_str().unwrap().to_string()
                    } else {
                        c.to_string()
                    }
                }).collect();
            }
        }

        if headers.is_empty() {
            if let Some(first_row) = data.first() {
                if let Some(arr) = first_row.as_array() {
                    headers = (0..arr.len()).map(|i| format!("col_{}", i)).collect();
                } else if let Some(obj) = first_row.as_object() {
                    headers = obj.keys().cloned().collect();
                }
            }
        }

        let mut rows: Vec<Vec<String>> = Vec::new();
        if let Some(first_row) = data.first() {
            if first_row.is_array() {
                for r in data {
                    if let Some(arr) = r.as_array() {
                        rows.push(arr.iter().map(|cell| {
                            if cell.is_null() {
                                String::new()
                            } else if cell.is_string() {
                                cell.as_str().unwrap().to_string()
                            } else {
                                cell.to_string()
                            }
                        }).collect());
                    }
                }
            } else if first_row.is_object() {
                for r in data {
                    if let Some(obj) = r.as_object() {
                        let mut row = Vec::new();
                        for h in &headers {
                            let cell = obj.get(h).unwrap_or(&Value::Null);
                            if cell.is_null() {
                                row.push(String::new());
                            } else if cell.is_string() {
                                row.push(cell.as_str().unwrap().to_string());
                            } else {
                                row.push(cell.to_string());
                            }
                        }
                        rows.push(row);
                    }
                }
            } else {
                let total = json.get("total")
                    .or_else(|| json.get("count"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(data.len() as i64);

                let mut lines = vec![format!("✅ **查询完成** ({}ms) | 共 {} 条", elapsed, total)];
                for (i, item) in data.iter().enumerate() {
                    lines.push(format!("{}. {}", i + 1, item));
                }
                return Ok(lines.join("\n"));
            }
        }

        let total = json.get("total")
            .or_else(|| json.get("count"))
            .and_then(|v| v.as_i64())
            .unwrap_or(data.len() as i64);

        let max_show = 200;
        let mut lines = vec![
            format!("✅ **查询完成** ({}ms) | 共 {} 行", elapsed, total),
            "".to_string(),
            format!("| {} |", headers.join(" | ")),
            format!("| {} |", headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")),
        ];

        let rows_to_show = std::cmp::min(rows.len(), max_show);
        for row in rows.iter().take(rows_to_show) {
            let clean_row: Vec<String> = row.iter().map(|c| c.replace('\n', " ")).collect();
            lines.push(format!("| {} |", clean_row.join(" | ")));
        }

        if rows.len() > max_show {
            lines.push("".to_string());
            lines.push(format!("*... 仅显示前 {} 行，共 {} 行*", max_show, total));
        }

        Ok(lines.join("\n"))
    }
}

async fn login(
    base_url: &str,
    username: &str,
    password: &str,
    timeout_ms: u64,
) -> Result<ArcherySession, String> {
    let clean_url = base_url.trim_end_matches('/');
    let jar = Arc::new(reqwest::cookie::Jar::default());
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .cookie_provider(jar.clone())
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let login_url = format!("{}/login/", clean_url);
    eprintln!("[Archery Client] Opening GET {}", login_url);
    let res = client.get(&login_url)
        .send()
        .await
        .map_err(|e| format!("GET /login/ request failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("GET /login/ returned HTTP {}", res.status()));
    }

    let parsed_url = clean_url.parse::<reqwest::Url>()
        .map_err(|e| format!("Failed to parse URL: {}", e))?;
    
    let mut csrf_token = String::new();
    if let Some(cookie_val) = jar.cookies(&parsed_url) {
        if let Ok(cookie_str) = cookie_val.to_str() {
            for part in cookie_str.split(';') {
                let parts: Vec<&str> = part.split('=').map(|s| s.trim()).collect();
                if parts.len() == 2 && parts[0] == "csrftoken" {
                    csrf_token = parts[1].to_string();
                    break;
                }
            }
        }
    }

    if csrf_token.is_empty() {
        let body = res.text().await.unwrap_or_default();
        if let Some(token) = extract_csrf_token(&body) {
            csrf_token = token;
        } else {
            return Err("csrftoken not found in cookies or HTML response".to_string());
        }
    }

    eprintln!("[Archery Client] CSRF token successfully retrieved: {}...", &csrf_token[..std::cmp::min(10, csrf_token.len())]);

    let auth_url = format!("{}/authenticate/", clean_url);
    eprintln!("[Archery Client] Posting authentication to {}", auth_url);
    let params = [
        ("username", username.to_string()),
        ("password", password.to_string()),
    ];

    let auth_res = client.post(&auth_url)
        .header("X-CSRFToken", &csrf_token)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Referer", &login_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("POST /authenticate/ request failed: {}", e))?;

    if !auth_res.status().is_success() {
        return Err(format!("POST /authenticate/ returned HTTP {}", auth_res.status()));
    }

    let text = auth_res.text().await
        .map_err(|e| format!("Failed to read authenticate response text: {}", e))?;

    let auth_json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Failed to parse JSON from /authenticate/: {} (body: {})", e, text))?;

    let status = auth_json.get("status").and_then(|s| s.as_i64()).unwrap_or(-1);
    if status != 0 {
        let msg = auth_json.get("msg").and_then(|m| m.as_str()).unwrap_or("Unknown authentication error");
        return Err(format!("Authentication failed (status {}): {}", status, msg));
    }

    let mut has_sessionid = false;
    if let Some(cookie_val) = jar.cookies(&parsed_url) {
        if let Ok(cookie_str) = cookie_val.to_str() {
            for part in cookie_str.split(';') {
                let parts: Vec<&str> = part.split('=').map(|s| s.trim()).collect();
                if parts.len() == 2 && parts[0] == "sessionid" {
                    has_sessionid = true;
                    break;
                }
            }
        }
    }

    if !has_sessionid {
        return Err("Login succeeded but sessionid cookie was not set in client jar".to_string());
    }

    eprintln!("[Archery Client] Successfully logged in and retrieved sessionid.");
    Ok(ArcherySession {
        base_url: clean_url.to_string(),
        client,
        jar,
        csrf_token,
    })
}

fn extract_csrf_token(html: &str) -> Option<String> {
    let marker = "csrfmiddlewaretoken";
    if let Some(pos) = html.find(marker) {
        let sub = &html[pos..];
        if let Some(val_pos) = sub.find("value=") {
            let val_sub = &sub[val_pos + 6..];
            let quote = val_sub.chars().next()?;
            if quote == '"' || quote == '\'' {
                if let Some(end_pos) = val_sub[1..].find(quote) {
                    return Some(val_sub[1..1 + end_pos].to_string());
                }
            }
        }
    }
    None
}

fn field_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^`([^`]+)`\s+([^\s,]+[^,]*?)(?:\s+(?:NOT\s+)?NULL)?(?:\s+AUTO_INCREMENT)?(?:\s+DEFAULT\s+[^,\s]+)?(?:\s+COMMENT\s+'([^']*)')?").unwrap()
    })
}
