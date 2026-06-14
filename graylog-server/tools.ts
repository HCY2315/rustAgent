/**
 * Epass Agent MCP - GrayLog 日志查询工具集
 *
 * 通过 Graylog REST API 直接操作，无需浏览器。
 * 使用 HTTP Basic Auth 进行认证。
 *
 * API:
 *   search_graylog → GET /api/search/universal/relative
 */

import "dotenv/config";
import { z } from "zod";
import type { ToolResult } from "../shared/types.js";

// ============================================================
// 通用错误提取
// ============================================================
function extractErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (typeof obj.message === "string") return obj.message;
    if (typeof obj.error === "string") return obj.error;
    if (typeof obj.reason === "string") return obj.reason;
    try {
      return JSON.stringify(err);
    } catch {
      return String(err);
    }
  }
  return String(err);
}

// ============================================================
// 环境变量
// ============================================================
const {
  GRAYLOG_URL,
  GRAYLOG_USERNAME,
  GRAYLOG_PASSWORD,
} = process.env;

function resolveAuth(args: {
  url?: string; username?: string; password?: string;
}): { url: string; username: string; password: string } {
  const url = args.url || GRAYLOG_URL || "";
  const username = args.username || GRAYLOG_USERNAME || "";
  const password = args.password || GRAYLOG_PASSWORD || "";

  const missing: string[] = [];
  if (!url) missing.push("url");
  if (!username) missing.push("username");
  if (!password) missing.push("password");
  if (missing.length > 0) {
    throw new Error(
      `缺少必要参数: ${missing.join(", ")}。请传入参数或在 .env 文件中配置 ` +
      `GRAYLOG_URL / GRAYLOG_USERNAME / GRAYLOG_PASSWORD`
    );
  }

  return { url, username, password };
}

// ============================================================
// HTTP Basic Auth 辅助
// ============================================================
function authHeader(username: string, password: string): string {
  return "Basic " + Buffer.from(`${username}:${password}`).toString("base64");
}

async function graylogFetch(
  baseUrl: string,
  username: string,
  password: string,
  path: string,
  params?: Record<string, string>,
): Promise<any> {
  const url = new URL(path, baseUrl.replace(/\/+$/, ""));
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      url.searchParams.set(k, v);
    }
  }

  const resp = await fetch(url.toString(), {
    method: "GET",
    headers: {
      "Accept": "application/json",
      "Authorization": authHeader(username, password),
    },
  });

  if (!resp.ok) {
    const text = await resp.text().catch(() => "");
    throw new Error(`Graylog API HTTP ${resp.status}: ${text.substring(0, 300)}`);
  }

  return resp.json();
}

// ============================================================
// 通用认证参数
// ============================================================
const AuthParams = {
  url: z.string().url().optional().describe("Graylog 服务地址，默认从 .env 的 GRAYLOG_URL 读取"),
  username: z.string().min(1).optional().describe("Graylog 登录用户名，默认从 .env 的 GRAYLOG_USERNAME 读取"),
  password: z.string().min(1).optional().describe("Graylog 登录密码，默认从 .env 的 GRAYLOG_PASSWORD 读取"),
};

// ============================================================
// 工具 1: 搜索日志消息（唯一工具）
// ============================================================
export const searchLogsTool = {
  name: "search_graylog",
  description: `在 Graylog 中搜索日志消息，支持 Lucene 查询语法和时间范围。

时间范围通过 range 参数控制（相对时间，单位秒）：
  - 最近 5 分钟 → range=300
  - 最近 1 小时 → range=3600
  - 最近 2 小时 → range=7200
  - 最近 1 天   → range=86400
  - 最近 7 天   → range=604800

常用 Lucene 查询示例:
  - "UpdateWorkOrder_DouyinCallback"       → 精确匹配
  - "ERROR"                                 → 搜索错误日志
  - "source:172.16.66.175"                  → 按来源搜索
  - "level:error AND message:timeout"       → 组合条件

支持通过 fields 参数指定返回哪些字段（逗号分隔），例如: "message,source,timestamp,level"。`,
  inputSchema: {
    type: "object",
    properties: {
      url: { type: "string", description: "Graylog 服务地址（可选，默认从 .env 读取）" },
      username: { type: "string", description: "Graylog 登录用户名（可选，默认从 .env 读取）" },
      password: { type: "string", description: "Graylog 登录密码（可选，默认从 .env 读取）" },
      query: { type: "string", description: "Lucene 查询语法，搜索关键词或表达式" },
      stream_id: { type: "string", description: "Stream ID（可选，默认 000000000000000000000001 全部消息）" },
      range: { type: "number", description: "相对时间范围（秒），默认 3600（1小时）" },
      limit: { type: "number", description: "返回结果条数，默认 20，最大 100" },
      fields: { type: "string", description: "返回字段，逗号分隔（可选，默认全部）" },
    },
    required: ["query"],
  },
};

export async function handleSearchLogs(args: unknown): Promise<ToolResult> {
  const schema = z.object({
    ...AuthParams,
    query: z.string().min(1, "查询词不能为空"),
    stream_id: z.string().optional().default("000000000000000000000001"),
    range: z.number().optional().default(3600),
    limit: z.number().optional().default(20),
    fields: z.string().optional(),
  });
  const parsed = schema.safeParse(args);
  if (!parsed.success) {
    return {
      content: [{ type: "text", text: `❌ 参数错误:\n${parsed.error.issues.map((i) => `  • ${i.path.join(".")}: ${i.message}`).join("\n")}` }],
      isError: true,
    };
  }

  const raw = parsed.data;
  let p: { url: string; username: string; password: string };
  try {
    p = resolveAuth(raw);
  } catch (e: any) {
    return { content: [{ type: "text", text: `❌ ${e.message}` }], isError: true };
  }

  try {
    const params: Record<string, string> = {
      query: raw.query,
      range: String(Math.min(raw.range, 2592000)), // 上限30天
      limit: String(Math.min(raw.limit, 100)),
      decorate: "false",
      sort: "timestamp:desc",
    };

    if (raw.stream_id) {
      params.filter = `streams:${raw.stream_id}`;
    }
    if (raw.fields) {
      params.fields = raw.fields;
    }

    const json = await graylogFetch(p.url, p.username, p.password, "/api/search/universal/relative", params);

    const messages = json.messages || [];
    const total = json.total_results || json.total || 0;
    const tookMs = json.time || 0;
    const fromTime = json.from || "";
    const toTime = json.to || "";
    const fieldsAvailable = json.fields || [];

    if (!Array.isArray(messages) || messages.length === 0) {
      const safeQuery = raw.query.length > 80 ? raw.query.substring(0, 80) + "..." : raw.query;
      let hint = "";
      if (raw.range < 7200) {
        hint = "。提示：当前时间范围较短，可尝试增大 range 参数（如 7200=2小时）";
      }
      return {
        content: [{
          type: "text",
          text: `✅ 查询完成 (${tookMs}ms)，共 ${total} 条结果\n\n` +
                `查询: ${safeQuery}\n时间范围: 过去 ${raw.range} 秒\n${hint}\n\n` +
                `未匹配到日志消息。`,
        }],
      };
    }

    // 格式化结果
    const maxShow = messages.length;
    const allFields = raw.fields
      ? raw.fields.split(",").map((f: string) => f.trim()).filter(Boolean)
      : ["timestamp", "source", "message", "level", "tag"];

    const lines: string[] = [
      `✅ **搜索完成** (${tookMs}ms) | 共 ${total} 条 | 显示前 ${maxShow} 条`,
      `查询: \`${raw.query}\``,
      `时间: ${fromTime} ~ ${toTime}`,
      ``,
    ];

    for (let i = 0; i < maxShow; i++) {
      const msg = messages[i]?.message || {};
      const ts = msg.timestamp || msg.time || "";
      const src = msg.source || "";
      const body = msg.message || "";

      lines.push(`---`);
      lines.push(`**#${i + 1}**  ${ts}  |  ${src}`);
      if (msg.tag) {
        lines.push(`标签: \`${msg.tag}\``);
      }
      if (body) {
        // 截断过长的消息体
        const truncated = body.length > 1200 ? body.substring(0, 1200) + "\n... (已截断)" : body;
        lines.push(``, truncated);
      }
      lines.push(``);
    }

    lines.push(`---`);
    lines.push(`💡 **提示**: 可通过 fields 参数自定义返回字段，或用更精确的 query 缩小范围`);
    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (err) {
    return {
      content: [{ type: "text", text: `❌ 搜索日志失败: ${extractErrorMessage(err)}` }],
      isError: true,
    };
  }
}

// ============================================================
// （handleSearchLogs / searchLogsTool 已在上方 export）
// ============================================================
