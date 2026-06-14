/**
 * Epass Agent MCP - Archery SQL 查询服务
 *
 * 提供 5 个独立工具，让 Agent 逐步引导用户完成数据库选择 → SQL 执行：
 *   1. list_archery_instances   → 列出所有数据库实例
 *   2. list_archery_databases   → 列出实例下的数据库
 *   3. list_archery_tables      → 列出数据库下的表
 *   4. describe_archery_table   → 查看表结构（DDL）
 *   5. execute_archery_sql      → 执行 SQL 查询
 *
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
  listInstancesTool, handleListInstances,
  listDatabasesTool, handleListDatabases,
  listTablesTool, handleListTables,
  describeTableTool, handleDescribeTable,
  executeSqlTool, handleExecuteSql,
} from "./tools.js";

const ALL_TOOLS = [
  listInstancesTool,
  listDatabasesTool,
  listTablesTool,
  describeTableTool,
  executeSqlTool,
];

// ============================================================
// MCP Server
// ============================================================
const server = new Server(
  {
    name: "Epass-archery-mcp",
    version: "0.1.0",
    description: "Archery SQL 审核查询平台 MCP 服务 - 5 个工具逐步引导数据库查询",
  },
  { capabilities: { tools: {} } }
);

// ============================================================
// 列出工具
// ============================================================
server.setRequestHandler(ListToolsRequestSchema, async () => {
  console.error(`[Archery MCP] ListTools (${ALL_TOOLS.length} 个工具)`);
  return { tools: ALL_TOOLS };
});

// ============================================================
// 调用工具
// ============================================================
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;
  console.error(`[Archery MCP] CallTool: ${name}`);

  try {
    switch (name) {
      case "list_archery_instances":
        return handleListInstances(args) as any;
      case "list_archery_databases":
        return handleListDatabases(args) as any;
      case "list_archery_tables":
        return handleListTables(args) as any;
      case "describe_archery_table":
        return handleDescribeTable(args) as any;
      case "execute_archery_sql":
        return handleExecuteSql(args) as any;
      default:
        throw new McpError(
          ErrorCode.MethodNotFound,
          `未知工具: "${name}"。可用工具: ${ALL_TOOLS.map((t) => t.name).join(", ")}`
        );
    }
  } catch (error) {
    const msg = error instanceof Error ? error.message : String(error);
    console.error(`[Archery MCP] 错误: ${msg}`);
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
  console.error("[Archery MCP] 正在关闭...");
}

process.on("SIGINT", async () => { await cleanup(); process.exit(0); });
process.on("SIGTERM", async () => { await cleanup(); process.exit(0); });
process.on("uncaughtException", async (err) => {
  console.error("[Archery MCP] 未捕获异常:", err);
  await cleanup();
  process.exit(1);
});
process.on("unhandledRejection", async (reason) => {
  console.error("[Archery MCP] 未处理拒绝:", reason);
  await cleanup();
  process.exit(1);
});

// ============================================================
// 启动
// ============================================================
async function main() {
  try {
    const transport = new StdioServerTransport();
    console.error("[Archery MCP] 正在启动...");
    await server.connect(transport);
    console.error("[Archery MCP] 已就绪");
  } catch (error) {
    console.error("[Archery MCP] 启动失败:", error);
    process.exit(1);
  }
}

main();
