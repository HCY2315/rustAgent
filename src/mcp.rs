use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// MCP Types
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}

pub type ToolHandler = Arc<
    dyn Fn(Value) -> BoxFuture<'static, Result<ToolResult, String>> + Send + Sync,
>;

pub struct McpTool {
    pub tool: Tool,
    pub handler: ToolHandler,
}

pub struct McpServer {
    name: String,
    version: String,
    tools: HashMap<String, McpTool>,
}

impl McpServer {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            tools: HashMap::new(),
        }
    }

    pub fn register_tool<F, Fut>(&mut self, name: &str, description: &str, schema: Value, handler: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolResult, String>> + Send + 'static,
    {
        let tool = Tool {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: schema,
        };
        let wrapped_handler = Arc::new(move |args| {
            let fut = handler(args);
            let boxed: BoxFuture<'static, Result<ToolResult, String>> = Box::pin(fut);
            boxed
        });
        self.tools.insert(
            name.to_string(),
            McpTool {
                tool,
                handler: wrapped_handler,
            },
        );
    }

    pub async fn run(&self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin).lines();

        eprintln!("[Archery MCP] Starting {} v{}...", self.name, self.version);

        while let Some(line) = reader.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                Ok(req) => {
                    if req.id.is_some() {
                        let resp = self.handle_request(req).await;
                        let resp_line = serde_json::to_string(&resp).unwrap();
                        println!("{}", resp_line);
                    } else {
                        // Notification
                        self.handle_notification(req).await;
                    }
                }
                Err(e) => {
                    eprintln!("[Archery MCP] Failed to parse request: {}", e);
                    let err_resp = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: Value::Null,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                            data: None,
                        }),
                    };
                    let resp_line = serde_json::to_string(&err_resp).unwrap();
                    println!("{}", resp_line);
                }
            }
        }

        eprintln!("[Archery MCP] Stdio stream closed, exiting.");
        Ok(())
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let id = req.id.clone().unwrap_or(Value::Null);
        match req.method.as_str() {
            "initialize" => {
                let result = serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": self.name,
                        "version": self.version
                    }
                });
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(result),
                    error: None,
                }
            }
            "tools/list" => {
                let tools_list: Vec<&Tool> = self.tools.values().map(|t| &t.tool).collect();
                let result = serde_json::json!({
                    "tools": tools_list
                });
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(result),
                    error: None,
                }
            }
            "tools/call" => {
                let tool_name = req.params.get("name").and_then(|v| v.as_str());
                let tool_args = req.params.get("arguments").cloned().unwrap_or(Value::Object(serde_json::Map::new()));

                if let Some(name) = tool_name {
                    if let Some(mcp_tool) = self.tools.get(name) {
                        let handler = mcp_tool.handler.clone();
                        match handler(tool_args).await {
                            Ok(res) => JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id,
                                result: Some(serde_json::to_value(res).unwrap()),
                                error: None,
                            },
                            Err(err_msg) => {
                                let fail_res = ToolResult {
                                    content: vec![ToolContent::Text {
                                        text: format!("❌ Tool execution failed: {}", err_msg),
                                    }],
                                    is_error: true,
                                };
                                JsonRpcResponse {
                                    jsonrpc: "2.0".to_string(),
                                    id,
                                    result: Some(serde_json::to_value(fail_res).unwrap()),
                                    error: None,
                                }
                            }
                        }
                    } else {
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32601,
                                message: format!("Tool not found: {}", name),
                                data: None,
                            }),
                        }
                    }
                } else {
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32602,
                            message: "Missing 'name' in tools/call parameters".to_string(),
                            data: None,
                        }),
                    }
                }
            }
            _ => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                }),
            },
        }
    }

    async fn handle_notification(&self, req: JsonRpcRequest) {
        if req.method == "notifications/initialized" {
            eprintln!("[Archery MCP] Client completed initialization handshake.");
        } else {
            eprintln!("[Archery MCP] Received unhandled notification: {}", req.method);
        }
    }
}
