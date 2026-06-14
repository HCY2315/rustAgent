/**
 * Epass Agent MCP - Archery SQL 查询工具集
 *
 * 通过 Archery REST API 操作，无需浏览器。
 * 提供 5 个独立工具，让 Agent 逐步引导用户完成选择。
 *
 * API 清单:
 *   1. list_archery_instances  → GET  /group/user_all_instances/
 *   2. list_archery_databases  → GET  /instance/instance_resource/?resource_type=database
 *   3. list_archery_tables     → GET  /instance/instance_resource/?resource_type=table
 *   4. describe_archery_table  → POST /instance/describetable/
 *   5. execute_archery_sql     → POST /query/
 */

import "dotenv/config";
import { z } from "zod";
import type { ToolResult } from "../shared/types.js";

// ============================================================
// 通用错误提取（兼容 ErrorEvent 等非标准错误）
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
// 环境变量（支持 .env 文件配置）
// ============================================================
const {
  ARCHERY_URL,
  ARCHERY_USERNAME,
  ARCHERY_PASSWORD,
} = process.env;

function resolveAuth(args: {
  url?: string; username?: string; password?: string; loginTimeout?: number;
}): { url: string; username: string; password: string; loginTimeout: number } {
  const url = args.url || ARCHERY_URL || "";
  const username = args.username || ARCHERY_USERNAME || "";
  const password = args.password || ARCHERY_PASSWORD || "";
  const loginTimeout = args.loginTimeout ?? 15000;

  const missing: string[] = [];
  if (!url) missing.push("url");
  if (!username) missing.push("username");
  if (!password) missing.push("password");
  if (missing.length > 0) {
    throw new Error(
      `缺少必要参数: ${missing.join(", ")}。请传入参数或在 .env 文件中配置 ` +
      `ARCHERY_URL / ARCHERY_USERNAME / ARCHERY_PASSWORD`
    );
  }

  return { url, username, password, loginTimeout };
}

// ============================================================
// 会话管理
// ============================================================
interface ArcherySession {
  baseUrl: string;
  cookies: Record<string, string>;
  csrfToken: string;
}

function cookieHeader(cookies: Record<string, string>): string {
  return Object.entries(cookies).map(([k, v]) => `${k}=${v}`).join("; ");
}

// ============================================================
// 会话缓存（避免每次工具调用都启动浏览器登录）
// ============================================================
let cachedSession: { session: ArcherySession; expiresAt: number } | null = null;

async function getSession(
  url: string, username: string, password: string, timeout: number
): Promise<ArcherySession> {
  // 检查缓存是否有效（30 分钟内有效）
  if (cachedSession && cachedSession.expiresAt > Date.now()) {
    // 验证 session 是否仍然有效
    const testResp = await fetch(
      `${cachedSession.session.baseUrl}/group/user_all_instances/?tag_codes%5B%5D=can_read`,
      {
        headers: {
          "Accept": "application/json",
          "X-CSRFToken": cachedSession.session.csrfToken,
          "X-Requested-With": "XMLHttpRequest",
          "Cookie": cookieHeader(cachedSession.session.cookies),
          "Referer": `${cachedSession.session.baseUrl}/sqlquery/`,
        },
      }
    );
    if (testResp.ok) {
      console.error(`[Archery] 使用缓存的会话 (${Math.round(cachedSession.expiresAt - Date.now()) / 60000}min 剩余)`);
      return cachedSession.session;
    }
    console.error(`[Archery] 缓存会话已过期，重新登录`);
  }
  cachedSession = null;

  const session = await loginToArchery(url, username, password, timeout);
  cachedSession = {
    session,
    expiresAt: Date.now() + 30 * 60 * 1000, // 30 分钟有效期
  };
  return session;
}

/**
 * 通过 Puppeteer 登录 Archery，获取登录态 Cookie
 *
 * Archery 登录使用 AJAX（按钮 type="button"），无法直接 fetch 模拟。
 * 使用 headless Chrome 模拟真实用户操作，获取 sessionid Cookie 后
 * 关闭浏览器页面，后续 API 调用使用 Cookie 认证。
 */
async function loginToArchery(
  url: string, username: string, password: string, timeout: number
): Promise<ArcherySession> {
  const baseUrl = url.replace(/\/+$/, "");

  const { createPage, closePage } = await import("../shared/browser.js");
  const page = await createPage();

  try {
    // ===== 打开登录页 =====
    console.error(`[Archery] 打开 ${baseUrl}/login/`);
    await page.goto(`${baseUrl}/login/`, {
      waitUntil: "networkidle2",
      timeout,
    });

    // ===== 检查是否已登录（可能浏览器有缓存的 session）=====
    const currentUrl = page.url();
    const isLoggedIn = !currentUrl.includes("/login");
    if (isLoggedIn) {
      console.error(`[Archery] 浏览器已有登录态，跳过登录 (${currentUrl})`);
    } else {
      // ===== 填写登录表单 =====
      await page.waitForSelector("#inputUsername", { visible: true, timeout });
      await page.type("#inputUsername", username, { delay: 20 });
      await page.type("#inputPassword", password, { delay: 20 });

      // ===== 点击登录按钮（触发 AJAX authenticateUser） =====
      console.error(`[Archery] 点击登录按钮...`);
      const navigationPromise = page.waitForNavigation({
        waitUntil: "networkidle2",
        timeout,
      }).catch(() => {});

      await page.click("#btnLogin");
      await navigationPromise;
      await new Promise((r) => setTimeout(r, 1000));
    }

    // ===== 提取 Cookies =====
    const cookiesList = await page.cookies();
    const cookies: Record<string, string> = {};
    for (const c of cookiesList) {
      cookies[c.name] = c.value;
    }

    const csrfToken = cookies["csrftoken"] || "";
    const sessionId = cookies["sessionid"] || "";

    if (!sessionId) {
      try {
        const { mkdirSync } = await import("fs");
        mkdirSync("./screenshots", { recursive: true });
        await page.screenshot({ path: `./screenshots/login_failed_${Date.now()}.png` });
      } catch { /* ignore */ }

      const pageUrl = page.url();
      const pageContent = await page.evaluate(() => document.body.innerText.substring(0, 300));
      throw new Error(
        `登录失败：未获取到 sessionid\n` +
        `当前页面: ${pageUrl}\n页面内容: ${pageContent}`
      );
    }

    console.error(`[Archery] sessionid=${sessionId.substring(0, 10)}...`);
    return { baseUrl, cookies, csrfToken };
  } finally {
    await closePage(page);
  }
}

// ============================================================
// 通用认证参数（所有字段可选，支持 .env 配置）
// ============================================================
const AuthParams = {
  url: z.string().url().optional().describe("Archery 服务地址，默认从 .env 的 ARCHERY_URL 读取"),
  username: z.string().min(1).optional().describe("Archery 登录用户名，默认从 .env 的 ARCHERY_USERNAME 读取"),
  password: z.string().min(1).optional().describe("Archery 登录密码，默认从 .env 的 ARCHERY_PASSWORD 读取"),
  loginTimeout: z.number().optional().default(15000).describe("登录超时(ms)"),
};

// ============================================================
// 工具 1: 列出数据库实例
// ============================================================
export const listInstancesTool = {
  name: "list_archery_instances",
  description: `列出当前用户在 Archery 上有权限访问的所有数据库实例。

首次使用 Archery 时应先调用此工具查看可用的数据库实例。
酒店类数据库实例以 "hotel_distribution" 开头（同程FP/同程SP/抖音SP/公共）。`,
  inputSchema: {
    type: "object",
    properties: {
      url: { type: "string", description: "Archery 服务地址（可选，默认从 .env 读取）" },
      username: { type: "string", description: "Archery 登录用户名（可选，默认从 .env 读取）" },
      password: { type: "string", description: "Archery 登录密码（可选，默认从 .env 读取）" },
      loginTimeout: { type: "number", description: "登录超时(ms)，默认 15000" },
    },
    required: [],
  },
};

export async function handleListInstances(args: unknown): Promise<ToolResult> {
  const schema = z.object(AuthParams);
  const parsed = schema.safeParse(args);
  if (!parsed.success) {
    return { content: [{ type: "text", text: `❌ 参数错误:\n${parsed.error.issues.map((i) => `  • ${i.path.join(".")}: ${i.message}`).join("\n")}` }], isError: true };
  }
  const raw = parsed.data;
  let p: { url: string; username: string; password: string; loginTimeout: number };
  try {
    p = resolveAuth(raw);
  } catch (e: any) {
    return { content: [{ type: "text", text: `❌ ${e.message}` }], isError: true };
  }
  try {
    const session = await getSession(p.url, p.username, p.password, p.loginTimeout);
    const resp = await fetch(`${session.baseUrl}/group/user_all_instances/?tag_codes%5B%5D=can_read`, {
      method: "GET",
      headers: {
        "Accept": "application/json, text/javascript, */*; q=0.01",
        "X-CSRFToken": session.csrfToken,
        "X-Requested-With": "XMLHttpRequest",
        "Cookie": cookieHeader(session.cookies),
        "Referer": `${session.baseUrl}/sqlquery/`,
      },
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const json = await resp.json();
    const instances = json.data || [];
    if (!Array.isArray(instances) || instances.length === 0) {
      return { content: [{ type: "text", text: "当前用户没有可访问的数据库实例。" }] };
    }

    // 实例别名映射
    const ALIASES: Record<string, string> = {
      "hotel_distribution": "同程FP酒店",
      "hotel_distribution_dy_sp": "抖音SP酒店",
      "hotel_distribution_public": "酒店-公共",
      "hotel_distribution_sp": "同程SP酒店",
    };
    function displayName(name: string): string {
      return ALIASES[name] ? `${name}（${ALIASES[name]}）` : name;
    }

    const hotel: string[] = [];
    const other: string[] = [];
    for (const inst of instances) {
      const name = inst.instance_name || inst.name || String(inst);
      if (name.startsWith("hotel_distribution")) hotel.push(name);
      else other.push(name);
    }
    const lines: string[] = [`✅ 登录成功，共 ${instances.length} 个可用数据库实例`, ``];
    if (hotel.length > 0) {
      lines.push(`🏨 **酒店类 (${hotel.length}个)**`);
      hotel.forEach((h) => lines.push(`   - ${displayName(h)}`));
      lines.push(``);
    }
    if (other.length > 0) {
      lines.push(`📦 **其他 (${other.length}个)**`);
      other.forEach((o) => lines.push(`   - ${o}`));
      lines.push(``);
    }
    lines.push(`---`);
    lines.push(`💡 **下一步**: 选择实例后，使用 list_archery_databases 查看其数据库列表`);
    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (err) {
    return { content: [{ type: "text", text: `❌ 获取实例列表失败: ${err instanceof Error ? err.message : String(err)}` }], isError: true };
  }
}

// ============================================================
// 工具 2: 列出数据库
// ============================================================
export const listDatabasesTool = {
  name: "list_archery_databases",
  description: `列出指定数据库实例下的所有数据库。

需先调用 list_archery_instances 获取 instance 名称。`,
  inputSchema: {
    type: "object",
    properties: {
      url: { type: "string", description: "Archery 服务地址" },
      username: { type: "string", description: "Archery 登录用户名" },
      password: { type: "string", description: "Archery 登录密码（可选，默认从 .env 读取）" },
      instance: { type: "string", description: "数据库实例名称" },
      loginTimeout: { type: "number", description: "登录超时(ms)，默认 15000" },
    },
    required: ["instance"],
  },
};

export async function handleListDatabases(args: unknown): Promise<ToolResult> {
  const schema = z.object({ ...AuthParams, instance: z.string().min(1).describe("数据库实例名称") });
  const parsed = schema.safeParse(args);
  if (!parsed.success) {
    return { content: [{ type: "text", text: `❌ 参数错误:\n${parsed.error.issues.map((i) => `  • ${i.path.join(".")}: ${i.message}`).join("\n")}` }], isError: true };
  }
  const raw = parsed.data;
  let p: { url: string; username: string; password: string; loginTimeout: number } & { instance: string };
  try {
    const auth = resolveAuth(raw);
    p = { ...auth, instance: raw.instance };
  } catch (e: any) {
    return { content: [{ type: "text", text: `❌ ${e.message}` }], isError: true };
  }
  try {
    const session = await getSession(p.url, p.username, p.password, p.loginTimeout);
    const url = `${session.baseUrl}/instance/instance_resource/?instance_name=${encodeURIComponent(p.instance)}&resource_type=database`;
    const resp = await fetch(url, {
      method: "GET",
      headers: {
        "Accept": "application/json, text/javascript, */*; q=0.01",
        "X-CSRFToken": session.csrfToken,
        "X-Requested-With": "XMLHttpRequest",
        "Cookie": cookieHeader(session.cookies),
        "Referer": `${session.baseUrl}/sqlquery/`,
      },
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const json = await resp.json();
    const databases = json.data || [];
    if (!Array.isArray(databases) || databases.length === 0) {
      return { content: [{ type: "text", text: `实例 "${p.instance}" 下没有数据库。` }] };
    }
    const lines = [
      `📂 **实例 "${p.instance}" 下的数据库 (${databases.length}个)**`,
      ``,
      ...databases.map((db: string) => `   - ${db}`),
      ``,
      `---`,
      `💡 **下一步**: 选择数据库后可使用:`,
      `   1. list_archery_tables → 查看表列表`,
      `   2. execute_archery_sql → 直接执行 SQL`,
    ];
    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (err) {
    return { content: [{ type: "text", text: `❌ 获取数据库列表失败: ${err instanceof Error ? err.message : String(err)}` }], isError: true };
  }
}

// ============================================================
// 工具 3: 列出表
// ============================================================
export const listTablesTool = {
  name: "list_archery_tables",
  description: `列出指定数据库下的所有数据表。

需先调用 list_archery_databases 获取 database 名称。`,
  inputSchema: {
    type: "object",
    properties: {
      url: { type: "string", description: "Archery 服务地址" },
      username: { type: "string", description: "Archery 登录用户名" },
      password: { type: "string", description: "Archery 登录密码（可选，默认从 .env 读取）" },
      instance: { type: "string", description: "数据库实例名称" },
      database: { type: "string", description: "数据库名称" },
      loginTimeout: { type: "number", description: "登录超时(ms)，默认 15000" },
    },
    required: ["instance", "database"],
  },
};

export async function handleListTables(args: unknown): Promise<ToolResult> {
  const schema = z.object({ ...AuthParams, instance: z.string().min(1), database: z.string().min(1) });
  const parsed = schema.safeParse(args);
  if (!parsed.success) {
    return { content: [{ type: "text", text: `❌ 参数错误:\n${parsed.error.issues.map((i) => `  • ${i.path.join(".")}: ${i.message}`).join("\n")}` }], isError: true };
  }
  const raw = parsed.data;
  let p: { url: string; username: string; password: string; loginTimeout: number } & { instance: string; database: string };
  try {
    const auth = resolveAuth(raw);
    p = { ...auth, instance: raw.instance, database: raw.database };
  } catch (e: any) {
    return { content: [{ type: "text", text: `❌ ${e.message}` }], isError: true };
  }
  try {
    const session = await getSession(p.url, p.username, p.password, p.loginTimeout);
    const url = `${session.baseUrl}/instance/instance_resource/?instance_name=${encodeURIComponent(p.instance)}&db_name=${encodeURIComponent(p.database)}&resource_type=table`;
    const resp = await fetch(url, {
      method: "GET",
      headers: {
        "Accept": "application/json, text/javascript, */*; q=0.01",
        "X-CSRFToken": session.csrfToken,
        "X-Requested-With": "XMLHttpRequest",
        "Cookie": cookieHeader(session.cookies),
        "Referer": `${session.baseUrl}/sqlquery/`,
      },
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const json = await resp.json();
    const tables = json.data || [];
    if (!Array.isArray(tables) || tables.length === 0) {
      return { content: [{ type: "text", text: `数据库 "${p.database}" 中没有表。` }] };
    }
    const lines = [
      `📋 **数据库 "${p.database}" 中的表 (${tables.length}个)**`,
      ``,
      ...tables.map((t: string) => `   - ${t}`),
      ``,
      `---`,
      `💡 **下一步**: 选择表后可使用:`,
      `   1. describe_archery_table → 查看表结构`,
      `   2. execute_archery_sql → 执行 SQL 查询`,
    ];
    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (err) {
    return { content: [{ type: "text", text: `❌ 获取表列表失败: ${err instanceof Error ? err.message : String(err)}` }], isError: true };
  }
}

// ============================================================
// 工具 4: 查看表结构
// ============================================================
export const describeTableTool = {
  name: "describe_archery_table",
  description: `查看指定表的字段结构、类型、注释等信息（通过 SHOW CREATE TABLE 获取 DDL）。`,
  inputSchema: {
    type: "object",
    properties: {
      url: { type: "string", description: "Archery 服务地址" },
      username: { type: "string", description: "Archery 登录用户名" },
      password: { type: "string", description: "Archery 登录密码（可选，默认从 .env 读取）" },
      instance: { type: "string", description: "数据库实例名称" },
      database: { type: "string", description: "数据库名称" },
      table: { type: "string", description: "表名" },
      loginTimeout: { type: "number", description: "登录超时(ms)，默认 15000" },
    },
    required: ["instance", "database", "table"],
  },
};

export async function handleDescribeTable(args: unknown): Promise<ToolResult> {
  const schema = z.object({ ...AuthParams, instance: z.string().min(1), database: z.string().min(1), table: z.string().min(1) });
  const parsed = schema.safeParse(args);
  if (!parsed.success) {
    return { content: [{ type: "text", text: `❌ 参数错误:\n${parsed.error.issues.map((i) => `  • ${i.path.join(".")}: ${i.message}`).join("\n")}` }], isError: true };
  }
  const raw = parsed.data;
  let p: { url: string; username: string; password: string; loginTimeout: number } & { instance: string; database: string; table: string };
  try {
    const auth = resolveAuth(raw);
    p = { ...auth, instance: raw.instance, database: raw.database, table: raw.table };
  } catch (e: any) {
    return { content: [{ type: "text", text: `❌ ${e.message}` }], isError: true };
  }
  try {
    const session = await getSession(p.url, p.username, p.password, p.loginTimeout);
    const body = new URLSearchParams({
      instance_name: p.instance, db_name: p.database, schema_name: "", tb_name: p.table,
    });
    const resp = await fetch(`${session.baseUrl}/instance/describetable/`, {
      method: "POST",
      headers: {
        "Accept": "application/json, text/javascript, */*; q=0.01",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRFToken": session.csrfToken,
        "X-Requested-With": "XMLHttpRequest",
        "Cookie": cookieHeader(session.cookies),
        "Referer": `${session.baseUrl}/sqlquery/`,
      },
      body,
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const json = await resp.json();
    if (json.status !== 0) {
      return { content: [{ type: "text", text: `❌ 获取表结构失败: ${json.msg || json.error || "未知错误"}` }], isError: true };
    }
    const rows = json.data?.rows || [];
    if (rows.length === 0) {
      return { content: [{ type: "text", text: `表 "${p.table}" 没有返回结构信息。` }] };
    }

    const lines: string[] = [
      `## 📐 表结构: ${p.table}`,
      ``,
      `**实例**: ${p.instance}`,
      `**数据库**: ${p.database}`,
      ``,
    ];

    // DDL
    for (const row of rows) {
      if (Array.isArray(row) && row.length >= 2) {
        lines.push("```sql");
        lines.push(row[1] as string);
        lines.push("```");
      }
    }

    // 字段摘要
    const ddlRow = rows.find((r: any) => Array.isArray(r) && r.length >= 2);
    if (ddlRow) {
      const ddl = ddlRow[1] as string;
      const fieldLines = ddl.split("\n").filter((l: string) =>
        l.trim().match(/^`[^`]+`/) && !l.trim().startsWith("PRIMARY") && !l.trim().startsWith("KEY") && !l.trim().startsWith("UNIQUE") && !l.trim().startsWith("CONSTRAINT")
      );
      if (fieldLines.length > 0) {
        lines.push(``, `**字段摘要 (${fieldLines.length}个)**`, ``);
        lines.push(`| 字段名 | 类型 | 注释 |`);
        lines.push(`| --- | --- | --- |`);
        for (const fl of fieldLines) {
          const match = fl.trim().match(/^`([^`]+)`\s+([^\s,]+[^,]*?)(?:\s+(NOT\s+)?NULL)?(?:\s+AUTO_INCREMENT)?(?:\s+DEFAULT\s+([^,\s]+))?(?:\s+COMMENT\s+'([^']*)')?/);
          if (match) {
            lines.push(`| \`${match[1]}\` | ${match[2].trim()} | ${match[5] || ""} |`);
          }
        }
      }
    }
    lines.push(``, `---`, `💡 **下一步**: 使用 execute_archery_sql 执行查询`);
    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (err) {
    return { content: [{ type: "text", text: `❌ 获取表结构失败: ${err instanceof Error ? err.message : String(err)}` }], isError: true };
  }
}

// ============================================================
// 工具 5: 执行 SQL
// ============================================================
export const executeSqlTool = {
  name: "execute_archery_sql",
  description: `在指定的数据库实例上执行 SQL 查询，返回结构化结果（Markdown 表格）。

支持 SELECT、SHOW、DESCRIBE 等查询语句。
酒店类数据库实例以 hotel_distribution 开头（同程FP/同程SP/抖音SP/公共）。`,
  inputSchema: {
    type: "object",
    properties: {
      url: { type: "string", description: "Archery 服务地址" },
      username: { type: "string", description: "Archery 登录用户名" },
      password: { type: "string", description: "Archery 登录密码（可选，默认从 .env 读取）" },
      instance: { type: "string", description: "数据库实例名称" },
      database: { type: "string", description: "数据库名称（可选，默认使用实例同名数据库）" },
      sql: { type: "string", description: "SQL 查询语句" },
      limit: { type: "number", description: "结果条数限制，默认 100" },
      loginTimeout: { type: "number", description: "登录超时(ms)，默认 15000" },
      queryTimeout: { type: "number", description: "查询超时(ms)，默认 60000" },
    },
    required: ["instance", "sql"],
  },
};

export async function handleExecuteSql(args: unknown): Promise<ToolResult> {
  const schema = z.object({
    ...AuthParams,
    instance: z.string().min(1),
    database: z.string().optional(),
    sql: z.string().min(1),
    limit: z.number().optional().default(100),
    queryTimeout: z.number().optional().default(60000),
  });
  const parsed = schema.safeParse(args);
  if (!parsed.success) {
    return { content: [{ type: "text", text: `❌ 参数错误:\n${parsed.error.issues.map((i) => `  • ${i.path.join(".")}: ${i.message}`).join("\n")}` }], isError: true };
  }
  const raw = parsed.data;
  let p: { url: string; username: string; password: string; loginTimeout: number } & { instance: string; database?: string; sql: string; limit: number; queryTimeout: number };
  try {
    const auth = resolveAuth(raw);
    p = { ...auth, instance: raw.instance, database: raw.database, sql: raw.sql, limit: raw.limit, queryTimeout: raw.queryTimeout };
  } catch (e: any) {
    return { content: [{ type: "text", text: `❌ ${e.message}` }], isError: true };
  }
  try {
    const session = await getSession(p.url, p.username, p.password, p.loginTimeout);
    const body = new URLSearchParams();
    body.append("instance_name", p.instance);
    body.append("db_name", p.database || p.instance);
    body.append("sql_content", p.sql);
    body.append("limit_num", String(p.limit));

    const startTime = Date.now();
    const resp = await fetch(`${session.baseUrl}/query/`, {
      method: "POST",
      headers: {
        "Accept": "application/json, text/javascript, */*; q=0.01",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRFToken": session.csrfToken,
        "X-Requested-With": "XMLHttpRequest",
        "Cookie": cookieHeader(session.cookies),
        "Referer": `${session.baseUrl}/sqlquery/`,
      },
      body,
      signal: AbortSignal.timeout(p.queryTimeout),
    });
    const elapsed = Date.now() - startTime;
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);

    const text = await resp.text();
    const contentType = resp.headers.get("content-type") || "";
    if (!contentType.includes("json")) {
      return { content: [{ type: "text", text: `✅ 查询完成 (${elapsed}ms)\n\n${text.substring(0, 2000)}` }] };
    }
    const json = JSON.parse(text);
    if (json.status === 1 || json.status === 2 || json.error || json.err) {
      return { content: [{ type: "text", text: `⚠️ SQL 执行错误: ${json.error || json.err || json.msg || "未知错误"}` }], isError: true };
    }

    // Archery /query/ API 返回格式: { data: { column_names: [...], rows: [[...], ...] } }
    const rawData = json.data || json.results || json;
    const rowsData = rawData?.rows ?? rawData;
        const columnNames = rawData?.column_list || rawData?.column_names || json.column_names || json.columns || json.headers || [];
    const data = Array.isArray(rowsData) ? rowsData : [];
    const total = json.total ?? json.count ?? data.length;
    if (!Array.isArray(data) || data.length === 0) {
      return { content: [{ type: "text", text: `✅ ${json.msg || "查询成功，未返回数据（0 条记录）。"}` }] };
    }

    const headers = Array.isArray(columnNames) && columnNames.length > 0
      ? columnNames.map(String)
      : Array.isArray(data[0])
        ? data[0].map((_: any, i: number) => `col_${i}`)
        : typeof data[0] === "object" ? Object.keys(data[0]) : [];

    let rows: string[][];
    if (Array.isArray(data[0])) {
      rows = data.map((r: any[]) => r.map((c: any) => String(c ?? "")));
    } else if (typeof data[0] === "object") {
      rows = data.map((row: any) => headers.map((h: string) => String(row[h] ?? "")));
    } else {
      return { content: [{ type: "text", text: `✅ 查询完成 (${elapsed}ms)，共 ${total} 条\n\n${data.map((r: any, i: number) => `${i + 1}. ${r}`).join("\n")}` }] };
    }

    const maxShow = 200;
    const lines = [
      `✅ **查询完成** (${elapsed}ms) | 共 ${total} 行`,
      ``,
      `| ${headers.join(" | ")} |`,
      `| ${headers.map(() => "---").join(" | ")} |`,
      ...rows.slice(0, maxShow).map((r) => `| ${r.map((c) => c.replace(/\n/g, " ")).join(" | ")} |`),
    ];
    if (rows.length > maxShow) lines.push(``, `*... 仅显示前 ${maxShow} 行，共 ${total} 行*`);
    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (err) {
    return { content: [{ type: "text", text: `❌ SQL 查询失败: ${err instanceof Error ? err.message : String(err)}` }], isError: true };
  }
}

export { getSession };
