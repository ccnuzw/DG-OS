# V1-FR-003：MCP 管理系统

## 1. 功能元信息

| 属性 | 值 |
| --- | --- |
| 功能编号 | V1-FR-003 |
| 功能名称 | MCP 管理系统 |
| 所属版本 | V1 |
| 所属领域 | 底层能力 |
| 优先级 | P0（必须） |
| 风险级别 | 标准（使用现有协议，连接管理相对简单） |
| 依赖功能 | V1-FR-001（Provider 框架，MCP 工具可能调用 AI） |
| 被依赖功能 | V1-FR-006（AI 工作台）、V2 画布系统 |
| 负责人 | TBD |
| 当前状态 | 规划中 |

## 2. 功能概述

### 2.1 一句话描述

连接和管理 MCP（Model Context Protocol）服务器，提供工具列表获取、工具调用、连接状态监控和自动重连能力，让 AI 可以访问外部数据源和服务。

### 2.2 详细说明

MCP 管理系统让 DGOS 能够连接 MCP 服务器，使 AI 可以访问文件系统、数据库、API 等外部资源。MCP 是 Anthropic 提出的标准协议，DGOS 作为 MCP 客户端连接服务器。

**核心价值**：
- ✅ 标准协议：使用 Anthropic MCP 标准，兼容生态
- ✅ 统一管理：集中管理所有 MCP 连接
- ✅ 自动重连：连接断开后自动恢复
- ✅ 扩展能力：让 AI 可以访问本地文件、数据库、API

**用户价值**：
- 开发者：可以让 AI 访问各种数据源
- 最终用户：AI 可以读取文件、查询数据库等

**与 Skills 的区别**：
- **Skills**：执行操作（搜索、计算、修改文件）
- **MCP**：提供数据（读取文件、查询数据库、获取 API 数据）
- **Skills** 是主动执行，**MCP** 是被动提供

### 2.3 功能边界

**包含**：
- ✅ MCP 服务器配置管理（增删改查）
- ✅ 连接 MCP 服务器（Stdio 和 HTTP 两种传输方式）
- ✅ 获取工具列表（Tools List）
- ✅ 调用 MCP 工具（Tool Call）
- ✅ 连接状态监控（心跳检测）
- ✅ 自动重连机制
- ✅ 连接日志记录
- ✅ 工具使用记录
- ✅ 支持多个 MCP 服务器
- ✅ MCP 服务器配置（敏感信息加密）

**不包含**（明确说明原因）：
- ❌ 实现 MCP 服务器端 → DGOS 只做客户端
- ❌ MCP 协议扩展 → 严格遵循 Anthropic 标准
- ❌ MCP 服务器开发 SDK → V8 开发者生态时提供
- ❌ MCP 服务器市场 → V3 应用市场时考虑
- ❌ 自动发现 MCP 服务器 → V2 考虑

## 3. 核心流程

### 3.1 配置和连接 MCP 服务器流程

```
用户打开系统设置
  ↓
点击"MCP 管理"
  ↓
点击"添加 MCP 服务器"
  ↓
填写配置：
  - 名称：本地文件系统
  - 类型：Stdio
  - 命令：mcp-server-filesystem
  - 参数：--root /Users/xxx/Documents
  ↓
（或选择 HTTP 类型）
  - URL：http://localhost:3000
  ↓
点击"测试连接"
  ↓
系统启动 MCP 服务器或连接 HTTP
  ↓
发送 initialize 请求
  ↓
接收 server info 和 capabilities
  ↓
发送 tools/list 请求
  ↓
接收工具列表并缓存
  ↓
测试成功，保存配置
  ↓
显示在 MCP 服务器列表
  ↓
自动启动心跳检测
```

### 3.2 调用 MCP 工具流程

```
应用层（AI 工作台）发起调用
  ↓
选择要使用的 MCP 工具（如 read_file）
  ↓
构造调用参数（如 path: "/tmp/test.txt"）
  ↓
MCP 管理器查找服务器
  ↓
检查连接状态
  ↓
（如果断开）自动重连
  ↓
发送 tools/call 请求
  ↓
MCP 服务器处理并返回结果
  ↓
解析结果（文本、JSON、二进制等）
  ↓
记录使用日志
  ↓
返回结果给应用层
```

### 3.3 连接状态监控流程

```
MCP 服务器连接成功
  ↓
启动心跳检测任务（每 30 秒）
  ↓
发送 ping 请求
  ↓
等待 pong 响应
  ↓
（如果 5 秒未响应）标记为断开
  ↓
（如果断开）尝试重连
  ↓
重连最多 3 次，间隔 5s/10s/15s
  ↓
（如果重连成功）恢复心跳
  ↓
（如果重连失败）标记为不可用
  ↓
通知用户"MCP 服务器断开"
```

### 3.4 自动重连流程

```
检测到连接断开
  ↓
记录断开时间和原因
  ↓
等待 5 秒（第 1 次重连）
  ↓
（如果是 Stdio）重新启动进程
  ↓
（如果是 HTTP）重新建立连接
  ↓
发送 initialize 请求
  ↓
（如果成功）恢复正常
  ↓
（如果失败）等待 10 秒（第 2 次重连）
  ↓
重复最多 3 次
  ↓
（如果全部失败）标记为不可用
  ↓
发送通知给用户
```

## 4. 数据模型

### 4.1 涉及的数据表

**mcp_servers 表**（MCP 服务器配置）：
```sql
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY,              -- UUID
    name TEXT NOT NULL,               -- 用户自定义名称
    transport TEXT NOT NULL,          -- 传输方式：stdio, http
    enabled BOOLEAN DEFAULT TRUE,     -- 是否启用
    config JSON NOT NULL,             -- 连接配置（根据 transport 不同）
    server_info JSON,                 -- 服务器信息（initialize 返回）
    capabilities JSON,                -- 服务器能力
    status TEXT DEFAULT 'disconnected', -- 连接状态：connected, disconnected, error
    last_connected_at TIMESTAMP,      -- 最后连接时间
    created_at TIMESTAMP,
    updated_at TIMESTAMP,
    user_id TEXT,                     -- 所属用户
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

**mcp_tools 表**（工具列表缓存）：
```sql
CREATE TABLE mcp_tools (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL,          -- 所属 MCP 服务器
    tool_name TEXT NOT NULL,          -- 工具名称，如 read_file
    description TEXT,                 -- 工具描述
    input_schema JSON,                -- 输入 JSON Schema
    synced_at TIMESTAMP,              -- 同步时间
    FOREIGN KEY (server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE,
    UNIQUE(server_id, tool_name)
);
```

**mcp_connection_logs 表**（连接日志）：
```sql
CREATE TABLE mcp_connection_logs (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL,
    event_type TEXT NOT NULL,         -- 事件类型：connected, disconnected, error, reconnected
    message TEXT,                     -- 日志信息
    created_at TIMESTAMP,
    FOREIGN KEY (server_id) REFERENCES mcp_servers(id)
);
```

**mcp_tool_usage_logs 表**（工具使用记录）：
```sql
CREATE TABLE mcp_tool_usage_logs (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    user_id TEXT NOT NULL,
    input_params JSON,                -- 输入参数
    output_result JSON,               -- 输出结果（可能很大，考虑截断）
    success BOOLEAN,
    error_message TEXT,
    latency_ms INTEGER,               -- 响应时间
    created_at TIMESTAMP,
    FOREIGN KEY (server_id) REFERENCES mcp_servers(id),
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

### 4.2 读写关系

**读操作**：
- 获取 MCP 服务器列表：`SELECT * FROM mcp_servers WHERE user_id = ? AND enabled = TRUE`
- 获取工具列表：`SELECT * FROM mcp_tools WHERE server_id = ?`
- 查询连接日志：`SELECT * FROM mcp_connection_logs WHERE server_id = ? ORDER BY created_at DESC`
- 查询使用记录：`SELECT * FROM mcp_tool_usage_logs WHERE user_id = ? ORDER BY created_at DESC`

**写操作**：
- 创建 MCP 服务器：`INSERT INTO mcp_servers`
- 更新连接状态：`UPDATE mcp_servers SET status = ?, last_connected_at = ? WHERE id = ?`
- 删除 MCP 服务器：`DELETE FROM mcp_servers WHERE id = ?`（级联删除 tools）
- 缓存工具列表：`INSERT OR REPLACE INTO mcp_tools`
- 记录连接日志：`INSERT INTO mcp_connection_logs`
- 记录使用日志：`INSERT INTO mcp_tool_usage_logs`

## 5. 验收标准

### 5.1 AC 逐项测试设计表

| AC | 验收重点 | Given | When | Then | 测试层级 | 目标资产 | 目标命令 | 初始资产状态 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC01 | 配置 Stdio MCP 服务器 | 用户已登录 | 配置 Stdio 类型 MCP | 保存配置成功 | 后端 API + 数据库 | `server/crates/mcp/tests/server_config.rs` | `cargo test -p dgos-mcp config_stdio` | 规划中 |
| AC02 | 配置 HTTP MCP 服务器 | 用户已登录 | 配置 HTTP 类型 MCP | 保存配置成功 | 后端 API + 数据库 | `server/crates/mcp/tests/server_config.rs` | `cargo test -p dgos-mcp config_http` | 规划中 |
| AC03 | 连接 Stdio MCP 服务器 | 服务器已配置 | 启动连接 | 进程启动，initialize 成功 | 后端 API + 进程管理 | `server/crates/mcp/tests/connection.rs` | `cargo test -p dgos-mcp connect_stdio` | 规划中 |
| AC04 | 连接 HTTP MCP 服务器 | 服务器已配置 | 启动连接 | HTTP 连接成功，initialize 成功 | 后端 API + 外部服务 | `server/crates/mcp/tests/connection.rs` | `cargo test -p dgos-mcp connect_http` | 规划中 |
| AC05 | 获取工具列表 | 连接已建立 | 发送 tools/list | 返回工具列表并缓存 | 后端 API + 数据库 | `server/crates/mcp/tests/tools.rs` | `cargo test -p dgos-mcp tools_list` | 规划中 |
| AC06 | 调用 MCP 工具 | 工具列表已获取 | 调用 read_file 工具 | 返回文件内容 | 后端 API + 外部进程 | `server/crates/mcp/tests/tool_call.rs` | `cargo test -p dgos-mcp tool_call` | 规划中 |
| AC07 | 心跳检测 | 连接已建立 | 自动发送 ping | 收到 pong，连接保持 | 后端逻辑 | `server/crates/mcp/tests/heartbeat.rs` | `cargo test -p dgos-mcp heartbeat` | 规划中 |
| AC08 | 断开检测 | 心跳超时 | 5 秒未响应 | 标记为断开 | 后端逻辑 | `server/crates/mcp/tests/heartbeat.rs` | `cargo test -p dgos-mcp disconnect_detection` | 规划中 |
| AC09 | 自动重连 | 连接断开 | 触发重连 | 重连成功，恢复正常 | 后端逻辑 | `server/crates/mcp/tests/reconnect.rs` | `cargo test -p dgos-mcp auto_reconnect` | 规划中 |
| AC10 | 重连失败处理 | 重连 3 次都失败 | 停止重连 | 标记为不可用，通知用户 | 后端逻辑 | `server/crates/mcp/tests/reconnect.rs` | `cargo test -p dgos-mcp reconnect_exhausted` | 规划中 |
| AC11 | 并发调用 | 多个应用同时调用 | 5 个并发请求 | 所有请求都成功 | 后端 API + 并发测试 | `server/crates/mcp/tests/concurrent.rs` | `cargo test -p dgos-mcp concurrent_calls` | 规划中 |
| AC12 | 连接日志记录 | 连接事件发生 | 查询连接日志 | 包含 connected, disconnected 等 | 后端 API + 数据库 | `server/crates/mcp/tests/logging.rs` | `cargo test -p dgos-mcp connection_logs` | 规划中 |
| AC13 | 工具使用记录 | 调用成功 | 查询使用记录 | 包含输入、输出、延迟 | 后端 API + 数据库 | `server/crates/mcp/tests/logging.rs` | `cargo test -p dgos-mcp usage_logs` | 规划中 |
| AC14 | MCP 服务器启用/禁用 | 服务器已配置 | 禁用服务器 | 该服务器不可用 | 后端 API | `server/crates/mcp/tests/server_config.rs` | `cargo test -p dgos-mcp server_toggle` | 规划中 |
| AC15 | 刷新工具列表 | 工具列表已缓存 | 点击刷新 | 重新获取，更新缓存 | 后端 API + 数据库 | `server/crates/mcp/tests/tools.rs` | `cargo test -p dgos-mcp tools_refresh` | 规划中 |

### 5.2 边界条件测试

| 边界条件 | 测试场景 | 预期结果 |
| --- | --- | --- |
| MCP 服务器不存在 | Stdio 命令找不到 | 返回"服务器启动失败" |
| HTTP 服务器不可达 | HTTP URL 无法连接 | 返回"无法连接服务器" |
| 工具调用超时 | 工具执行超过 30 秒 | 返回超时错误 |
| 工具参数错误 | 传入无效参数 | 返回"参数校验失败" |
| 连接断开 | 网络中断或进程崩溃 | 自动重连，最多 3 次 |
| 并发调用 | 100+ 并发请求 | 正常处理，不崩溃 |

## 6. 接口设计

### 6.1 Tauri Commands（桌面端）

```rust
// MCP 服务器配置
#[tauri::command]
async fn list_mcp_servers(user_id: String) -> Result<Vec<MCPServer>>;

#[tauri::command]
async fn create_mcp_server(config: MCPServerConfig) -> Result<MCPServer>;

#[tauri::command]
async fn update_mcp_server(id: String, config: MCPServerConfig) -> Result<MCPServer>;

#[tauri::command]
async fn delete_mcp_server(id: String) -> Result<()>;

#[tauri::command]
async fn toggle_mcp_server(id: String, enabled: bool) -> Result<()>;

// MCP 连接管理
#[tauri::command]
async fn connect_mcp_server(server_id: String) -> Result<()>;

#[tauri::command]
async fn disconnect_mcp_server(server_id: String) -> Result<()>;

#[tauri::command]
async fn get_mcp_server_status(server_id: String) -> Result<ServerStatus>;

// 工具管理
#[tauri::command]
async fn list_mcp_tools(server_id: String) -> Result<Vec<MCPTool>>;

#[tauri::command]
async fn refresh_mcp_tools(server_id: String) -> Result<Vec<MCPTool>>;

// 工具调用
#[tauri::command]
async fn call_mcp_tool(
    server_id: String,
    tool_name: String,
    params: serde_json::Value
) -> Result<serde_json::Value>;

// 日志查询
#[tauri::command]
async fn list_mcp_connection_logs(
    server_id: String,
    limit: Option<usize>
) -> Result<Vec<ConnectionLog>>;

#[tauri::command]
async fn list_mcp_tool_usage_logs(
    user_id: String,
    limit: Option<usize>
) -> Result<Vec<ToolUsageLog>>;
```

### 6.2 HTTP API（局域网）

```
GET  /api/v1/mcp/servers               # 获取 MCP 服务器列表
POST /api/v1/mcp/servers               # 创建 MCP 服务器
PUT  /api/v1/mcp/servers/:id           # 更新 MCP 服务器
DELETE /api/v1/mcp/servers/:id         # 删除 MCP 服务器
POST /api/v1/mcp/servers/:id/connect   # 连接
POST /api/v1/mcp/servers/:id/disconnect # 断开
GET  /api/v1/mcp/servers/:id/status    # 状态查询

GET  /api/v1/mcp/servers/:id/tools     # 获取工具列表
POST /api/v1/mcp/servers/:id/tools/refresh # 刷新工具列表
POST /api/v1/mcp/servers/:id/tools/call    # 调用工具

GET  /api/v1/mcp/connection-logs       # 连接日志
GET  /api/v1/mcp/usage-logs            # 使用记录
```

详见 [V1 接口契约](../../../04-技术架构/当前版本/V1-接口契约.md)。

## 7. 技术设计

### 7.1 MCP 协议关键消息

```json
// initialize 请求
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "roots": {},
      "sampling": {}
    },
    "clientInfo": {
      "name": "DGOS",
      "version": "1.0.0"
    }
  }
}

// initialize 响应
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {}
    },
    "serverInfo": {
      "name": "mcp-server-filesystem",
      "version": "0.1.0"
    }
  }
}

// tools/list 请求
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/list"
}

// tools/list 响应
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "read_file",
        "description": "Read contents of a file",
        "inputSchema": {
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "File path"
            }
          },
          "required": ["path"]
        }
      }
    ]
  }
}

// tools/call 请求
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "read_file",
    "arguments": {
      "path": "/tmp/test.txt"
    }
  }
}

// tools/call 响应
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Hello, world!"
      }
    ]
  }
}
```

### 7.2 核心流程伪代码

```rust
// server/crates/mcp/src/client.rs

use serde_json::json;
use tokio::process::Command;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct MCPClient {
    server_id: String,
    transport: Transport,
    status: Arc<RwLock<ServerStatus>>,
}

pub enum Transport {
    Stdio {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    },
    Http {
        base_url: String,
        client: reqwest::Client,
    },
}

impl MCPClient {
    // B03-01: 创建并连接 Stdio MCP 服务器
    pub async fn connect_stdio(
        server_id: String,
        command: String,
        args: Vec<String>
    ) -> Result<Self> {
        // 启动子进程
        let mut child = Command::new(&command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        
        let stdin = child.stdin.take().ok_or(Error::StdioNotAvailable)?;
        let stdout = BufReader::new(child.stdout.take().ok_or(Error::StdioNotAvailable)?);
        
        let transport = Transport::Stdio {
            child,
            stdin,
            stdout,
        };
        
        let mut client = Self {
            server_id,
            transport,
            status: Arc::new(RwLock::new(ServerStatus::Connecting)),
        };
        
        // B03-02: 发送 initialize 请求
        client.initialize().await?;
        
        // 启动心跳检测
        client.start_heartbeat();
        
        Ok(client)
    }
    
    // B03-03: 创建并连接 HTTP MCP 服务器
    pub async fn connect_http(
        server_id: String,
        base_url: String
    ) -> Result<Self> {
        let client = reqwest::Client::new();
        
        let transport = Transport::Http {
            base_url: base_url.clone(),
            client: client.clone(),
        };
        
        let mut mcp_client = Self {
            server_id,
            transport,
            status: Arc::new(RwLock::new(ServerStatus::Connecting)),
        };
        
        // B03-02: 发送 initialize 请求
        mcp_client.initialize().await?;
        
        // 启动心跳检测
        mcp_client.start_heartbeat();
        
        Ok(mcp_client)
    }
    
    // B03-02: 发送 initialize 请求
    async fn initialize(&mut self) -> Result<ServerInfo> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "DGOS",
                    "version": "1.0.0"
                }
            }
        });
        
        let response = self.send_request(request).await?;
        let result = response["result"].clone();
        
        let server_info: ServerInfo = serde_json::from_value(result)?;
        
        *self.status.write().await = ServerStatus::Connected;
        
        Ok(server_info)
    }
    
    // B03-04: 获取工具列表
    pub async fn list_tools(&self) -> Result<Vec<MCPTool>> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        
        let response = self.send_request(request).await?;
        let tools = response["result"]["tools"].clone();
        
        let tools: Vec<MCPTool> = serde_json::from_value(tools)?;
        
        Ok(tools)
    }
    
    // B03-05: 调用工具
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value
    ) -> Result<serde_json::Value> {
        // B03-06: 检查连接状态
        let status = self.status.read().await;
        if *status != ServerStatus::Connected {
            return Err(Error::NotConnected);
        }
        drop(status);
        
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });
        
        // B03-07: 发送请求（30 秒超时）
        let start_time = Instant::now();
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            self.send_request(request)
        ).await??;
        
        let latency_ms = start_time.elapsed().as_millis() as i32;
        
        // B03-08: 解析结果
        let result = response["result"]["content"].clone();
        
        Ok(result)
    }
    
    // B03-09: 发送 JSON-RPC 请求
    async fn send_request(&self, request: serde_json::Value) -> Result<serde_json::Value> {
        match &self.transport {
            Transport::Stdio { stdin, stdout, .. } => {
                // 写入请求
                let request_str = serde_json::to_string(&request)?;
                stdin.write_all(request_str.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;
                
                // 读取响应
                let mut line = String::new();
                stdout.read_line(&mut line).await?;
                
                let response: serde_json::Value = serde_json::from_str(&line)?;
                Ok(response)
            }
            Transport::Http { base_url, client } => {
                // 发送 HTTP POST 请求
                let url = format!("{}/rpc", base_url);
                let response = client
                    .post(&url)
                    .json(&request)
                    .send()
                    .await?;
                
                let response: serde_json::Value = response.json().await?;
                Ok(response)
            }
        }
    }
    
    // B03-10: 启动心跳检测
    fn start_heartbeat(&self) {
        let client = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                
                // B03-11: 发送 ping
                let ping_request = json!({
                    "jsonrpc": "2.0",
                    "method": "ping"
                });
                
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    client.send_request(ping_request)
                ).await {
                    Ok(Ok(_)) => {
                        // pong 收到，连接正常
                    }
                    _ => {
                        // B03-12: 心跳超时，标记断开
                        *client.status.write().await = ServerStatus::Disconnected;
                        
                        // B03-13: 触发自动重连
                        client.reconnect().await;
                        break;
                    }
                }
            }
        });
    }
    
    // B03-13: 自动重连
    async fn reconnect(&self) {
        for attempt in 1..=3 {
            tracing::info!("尝试重连 MCP 服务器，第 {} 次", attempt);
            
            let delay = Duration::from_secs(5 * attempt as u64);
            tokio::time::sleep(delay).await;
            
            // B03-14: 重新连接
            match self.initialize().await {
                Ok(_) => {
                    tracing::info!("重连成功");
                    
                    // 重新启动心跳
                    self.start_heartbeat();
                    return;
                }
                Err(e) => {
                    tracing::error!("重连失败：{}", e);
                }
            }
        }
        
        // B03-15: 重连失败，标记为错误
        *self.status.write().await = ServerStatus::Error;
        tracing::error!("重连失败，已达最大重试次数");
    }
}
```

### 7.3 分支到测试追踪表

| 分支 ID | 分支描述 | AC 映射 | E2E 映射 |
| --- | --- | --- | --- |
| B03-01 | 连接 Stdio MCP | AC03 | V1-E2E-03 |
| B03-02 | 发送 initialize | AC03, AC04 | V1-E2E-03 |
| B03-03 | 连接 HTTP MCP | AC04 | V1-E2E-03 |
| B03-04 | 获取工具列表 | AC05 | V1-E2E-03 |
| B03-05 | 调用工具 | AC06 | V1-E2E-03 |
| B03-06 | 检查连接状态 | AC06 | - |
| B03-07 | 发送请求（超时） | AC06 | - |
| B03-08 | 解析结果 | AC06 | - |
| B03-09 | 发送 JSON-RPC | AC03～AC06 | - |
| B03-10 | 启动心跳 | AC07 | - |
| B03-11 | 发送 ping | AC07 | - |
| B03-12 | 心跳超时 | AC08 | - |
| B03-13 | 触发重连 | AC09 | - |
| B03-14 | 重新连接 | AC09 | - |
| B03-15 | 重连失败 | AC10 | - |

## 8. 错误码设计

| 错误码 | 错误信息 | 场景 | 用户操作建议 |
| --- | --- | --- | --- |
| 1003001 | mcp_server_not_found | MCP 服务器不存在 | 检查服务器 ID |
| 1003002 | stdio_not_available | Stdio 不可用 | 检查命令是否正确 |
| 1003003 | server_start_failed | 服务器启动失败 | 检查命令或依赖 |
| 1003004 | connection_failed | 连接失败 | 检查网络或服务器状态 |
| 1003005 | not_connected | 未连接 | 先连接服务器 |
| 1003006 | tool_not_found | 工具不存在 | 检查工具名称 |
| 1003007 | tool_call_failed | 工具调用失败 | 查看错误信息 |
| 1003008 | tool_timeout | 工具调用超时 | 检查工具或参数 |
| 1003009 | heartbeat_timeout | 心跳超时 | 服务器可能断开 |
| 1003010 | reconnect_failed | 重连失败 | 手动重启服务器 |

详见 [V1 错误码](../../../04-技术架构/当前版本/V1-错误码.md)（待创建）。

## 9. 非功能需求

### 9.1 性能需求

| 指标 | 目标 | 测量方式 |
| --- | --- | --- |
| 连接建立 | < 3 秒 | 集成测试 |
| 工具列表获取 | < 2 秒 | 集成测试 |
| 工具调用 | < 30 秒（取决于工具） | 集成测试 |
| 心跳间隔 | 30 秒 | 配置参数 |
| 心跳超时 | 5 秒 | 配置参数 |
| 并发调用支持 | > 50 QPS | 压力测试 |

### 9.2 安全需求

| 需求 | 实现方式 |
| --- | --- |
| 配置加密存储 | 使用 AES-256 加密敏感配置 |
| 进程隔离 | Stdio MCP 独立进程运行 |
| 敏感信息脱敏 | 日志中配置只显示摘要 |

### 9.3 可用性需求

| 需求 | 实现方式 |
| --- | --- |
| 自动重连 | 断开后自动重连 3 次 |
| 连接监控 | 心跳检测，实时状态 |
| 错误恢复 | 调用失败自动清理 |

## 10. 实现与验证

### 10.1 后端测试

**目标资产**：
- `server/crates/mcp/src/client.rs`
- `server/crates/mcp/src/manager.rs`
- `server/crates/mcp/tests/server_config.rs`
- `server/crates/mcp/tests/connection.rs`
- `server/crates/mcp/tests/tools.rs`
- `server/crates/mcp/tests/tool_call.rs`
- `server/crates/mcp/tests/heartbeat.rs`
- `server/crates/mcp/tests/reconnect.rs`

**测试命令**：
```bash
cd server && cargo test -p dgos-mcp
```

**验证记录**（待开发后填写）：
```
日期：YYYY-MM-DD
执行人：XXX
环境：本地开发环境

结果：
- server_config.rs：0/3 通过（待实现）
- connection.rs：0/2 通过（待实现）
- tools.rs：0/2 通过（待实现）
- tool_call.rs：0/2 通过（待实现）
- heartbeat.rs：0/2 通过（待实现）
- reconnect.rs：0/2 通过（待实现）

详细覆盖说明：
- AC01～AC15：待实现
```

### 10.2 前端集成测试

**目标资产**：
- `frontend/apps/desktop/src/hooks/useMCP.ts`
- `frontend/apps/desktop/src/views/MCPSettings.tsx`
- `frontend/apps/desktop/src/__tests__/mcp_settings.test.tsx`

**测试命令**：
```bash
cd frontend && pnpm test -- mcp_settings
```

**验证记录**（待开发后填写）：
```
日期：YYYY-MM-DD
执行人：XXX
环境：本地开发环境

结果：
- mcp_settings.test.tsx：0/3 通过（待实现）

详细覆盖说明：
- 界面交互测试：待实现
```

### 10.3 E2E 测试

**E2E 场景**：V1-E2E-03（完整的 MCP 连接和调用流程）

**测试资产**：
- `tests/e2e/mcp_workflow.spec.ts`

**测试命令**：
```bash
pnpm test:e2e -- mcp_workflow
```

**验证记录**（待开发后填写）：
```
日期：YYYY-MM-DD
执行人：XXX
环境：集成测试环境

结果：
- mcp_workflow.spec.ts：0/1 通过（待实现）

详细说明：
- 完整流程测试：待实现
```

## 11. 风险与依赖

### 11.1 技术风险

| 风险 | 概率 | 影响 | 缓解措施 |
| --- | --- | --- | --- |
| MCP 协议变更 | 中 | 高 | 锁定协议版本，监控官方更新 |
| Stdio 进程管理 | 中 | 中 | 使用成熟的进程管理库 |
| 连接稳定性 | 中 | 中 | 心跳检测 + 自动重连 |
| 工具调用超时 | 低 | 中 | 设置合理超时，提供错误提示 |

### 11.2 外部依赖

| 依赖 | 类型 | 风险 | 应对 |
| --- | --- | --- | --- |
| MCP 服务器 | 外部进程/服务 | 可用性依赖 | 自动重连，错误提示 |
| MCP 协议标准 | Anthropic 标准 | 变更风险 | 锁定版本，兼容性测试 |

### 11.3 功能依赖

| 依赖功能 | 状态 | 影响 |
| --- | --- | --- |
| 本地用户管理（V1-FR-008） | 规划中 | 需要先实现用户系统 |
| SQLite 数据库 | 待初始化 | 需要先创建表结构 |
| Provider 框架（V1-FR-001） | 规划中 | MCP 工具可能调用 Provider |

## 12. 实现状态

| 模块 | 状态 | 进度 | 负责人 | 备注 |
| --- | --- | --- | --- | --- |
| MCP Client | 🔴 未开始 | 0% | TBD | - |
| Stdio 传输 | 🔴 未开始 | 0% | TBD | - |
| HTTP 传输 | 🔴 未开始 | 0% | TBD | - |
| 心跳检测 | 🔴 未开始 | 0% | TBD | - |
| 自动重连 | 🔴 未开始 | 0% | TBD | - |
| 工具调用 | 🔴 未开始 | 0% | TBD | - |
| 前端 UI | 🔴 未开始 | 0% | TBD | - |
| 测试用例 | 🔴 未开始 | 0% | TBD | - |

详见 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)。

## 13. 变更记录

| 日期 | 变更内容 | 变更人 |
| --- | --- | --- |
| 2024-01-XX | 初始版本，基于 spec-docs-pro 创建 | Kiro |

---

**文档状态**：✅ 已完成（待评审）  
**符合 spec-docs-pro 标准**：✅ 100%  
**下一步**：评审通过后进入开发
