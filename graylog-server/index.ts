/**
 * Epass Agent MCP - GrayLog 日志查询服务
 *
 * 提供 2 个独立工具，让 Agent 快速检索 Graylog 日志：
 *   1. list_graylog_streams → 列出所有日志流
 *   2. search_graylog      → 按 Lucene 查询搜索日志
 *
 * 适配版本: Graylog v3.1.4
 * 认证方式: HTTP Basic Auth
 * API 基础地址: http://47.102.148.195:9000/api
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
  ErrorCode,
  McpError,
} from "@modelcontextprotocol/sdk/types.js";
import {
  searchLogsTool, handleSearchLogs,
} from "./tools.js";

const ALL_TOOLS = [
  searchLogsTool,
];

// ============================================================
// MCP Server
// ============================================================
const server = new Server(
  {
    name: "Epass-graylog-mcp",
    version: "0.1.0",
    description: "GrayLog 日志查询 MCP 服务 - 支持 Lucene 语法搜索（仅 All messages Stream）",
  },
  { capabilities: { tools: {} } }
);

// ============================================================
// 列出工具
// ============================================================
server.setRequestHandler(ListToolsRequestSchema, async () => {
  console.error(`[Graylog MCP] ListTools (${ALL_TOOLS.length} 个工具)`);
  return { tools: ALL_TOOLS };
});

// ============================================================
// 调用工具
// ============================================================
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;
  console.error(`[Graylog MCP] CallTool: ${name}`);

  try {
    switch (name) {
      case "search_graylog":
        return handleSearchLogs(args) as any;
      default:
        throw new McpError(
          ErrorCode.MethodNotFound,
          `未知工具: "${name}"。可用工具: ${ALL_TOOLS.map((t) => t.name).join(", ")}`
        );
    }
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    console.error(`[Graylog MCP] 错误: ${msg}`);
    return {
      content: [{ type: "text" as const, text: `❌ 工具执行失败: ${msg}` }],
      isError: true,
    } as any;
  }
});

// ============================================================
// 生命周期
// ============================================================
async function cleanup() {
  console.error("[Graylog MCP] 正在关闭...");
}

process.on("SIGINT", async () => { await cleanup(); process.exit(0); });
process.on("SIGTERM", async () => { await cleanup(); process.exit(0); });
process.on("uncaughtException", async (err) => {
  console.error("[Graylog MCP] 未捕获异常:", err);
  await cleanup();
  process.exit(1);
});
process.on("unhandledRejection", async (reason) => {
  console.error("[Graylog MCP] 未处理拒绝:", reason);
  await cleanup();
  process.exit(1);
});

// ============================================================
// 启动
// ============================================================
async function main() {
  try {
    const transport = new StdioServerTransport();
    console.error("[Graylog MCP] 正在启动...");
    await server.connect(transport);
    console.error("[Graylog MCP] 已就绪");
  } catch (error) {
    console.error("[Graylog MCP] 启动失败:", error);
    process.exit(1);
  }
}

main();