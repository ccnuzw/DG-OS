# V1 MCP 管理系统技术设计

| 属性 | 值 |
| --- | --- |
| 日期 / 范围 | 2026-10-27 / V1-FR-003 实施基线 |
| 需求依据 | [V1-FR-003 MCP 管理系统](../../03-功能规格/V1/01-底层能力/03-MCP管理系统.md)、[D003](../../06-决策记录/V1-冻结决策.md#d003-不使用-quickjsnodejs-作为-skillsmcp-运行时)、[D005](../../06-决策记录/V1-冻结决策.md#d005-skills-和-mcp-使用现有标准协议) |
| 设计状态 | V1 边界已冻结（D021）；当前实现证据与剩余差距见实现状态及功能文档第 10 节 |
| 实现状态 | 只在 [V1 实现状态](../../02-产品与版本/当前版本/V1-实现状态.md) 维护 |

> 本文是 V1-FR-003 的实施基线。D021/D022 冻结传输、自动连接、逐次确认、凭据、审计和显式工作台消费默认值；具体字段以接口契约和数据模型为权威来源，测试证据在功能文档第 10 节维护。

## 1. 设计目标与边界

### 1.1 V1 目标

DGOS 作为 MCP Client，在本地受控地管理多个 MCP Server，完成：

1. 用户配置、编辑、启用和删除 MCP Server。
2. 通过 `stdio` 或 HTTP 连接并完成 MCP 初始化握手。
3. 发现并缓存 Server 的工具清单及输入 schema。
4. 在权限、超时和取消约束下调用指定工具。
5. 提供连接状态、错误摘要、重连和脱敏使用记录。
6. 为 AI Workbench 提供“可用工具描述”和“受控工具调用”桥接，但不把 MCP 工具自动注入所有任务。

### 1.2 V1 不做

- 不实现 MCP Server，不嵌入 QuickJS/Node.js，不执行 Skill 或 MCP 附带脚本。
- 不做 MCP Server 市场、自动发现、远程安装或任意用户自定义 registry。
- 不把 MCP 工具描述当作宿主权限；工具调用仍受用户、应用和模块权限检查。
- 不在数据库中保存 API key、OAuth token 或完整工具输入/输出。
- 原管理切片不接入单任务工作台；独立验证后的消费扩展按 D022 和[工作台技能消费设计](V1-工作台技能消费设计.md#5-mcp-显式消费扩展d022)执行，不改变无工具默认值。

## 2. 已冻结的关键决策

| 决策点 | V1 基线 | 原因 |
| --- | --- | --- |
| 传输 | 用户配置只暴露 `stdio` 与 `http`；HTTP 适配器优先使用 Streamable HTTP，初始化收到 400/404/405 时内部兼容 legacy SSE GET/POST 双通道 | 兼顾当前生态与后续协议演进，不把 legacy SSE 暴露为额外用户配置类型 |
| 连接默认值 | 保存配置不自动启动；提供“连接”与可选 `auto_connect` 开关，默认关闭 | 避免启动时执行未知命令或访问外部地址 |
| stdio 命令 | 使用可执行文件路径 + argv 数组 + 受控 cwd，不经过 shell | 防止 shell 注入、参数重解释和隐式管道 |
| HTTP 凭据 | 仅引用宿主凭据服务的 `credential_ref`；不允许在普通配置 JSON 中写 secret | 与 Provider 凭据边界一致，日志也不回显 header |
| 工具调用 | 每次调用检查 `mcp.call`、Server 启用状态、工具存在性和输入 schema；高风险工具由宿主确认 | MCP 内容不能自行授予权限 |
| 自动重连 | 仅对曾成功连接的 Server 做有界指数退避；用户主动断开后不重连 | 防止重试风暴和用户意图被覆盖 |
| HTTP 网络范围 | V1 默认允许 HTTPS 与 localhost；明文远程 HTTP 需用户显式确认 | 降低凭据被窃取和局域网劫持风险 |

上述方案已由 [D021](../../06-决策记录/V1-冻结决策.md#d021-mcp-v1-客户端边界与安全默认值) 冻结。后续实现若要改变这些默认值，必须追加新的决策并给出迁移/兼容方案。

## 3. 架构

```text
React MCP 管理页 / AI Workbench
              |
        Tauri Commands
              |
        dgos-mcp::McpManager
      /          |             \
  ConfigStore  ConnectionPool  Policy/Audit
      |          |               |
  SQLite      McpClient       权限/凭据服务
                   |
          Transport (stdio/http)
                   |
              MCP Server
```

### 3.1 Rust 模块职责

已落地的 `crates/dgos-mcp` 模块职责如下：

| 模块 | 职责 |
| --- | --- |
| `config` | 解析和校验 Server 配置；拒绝 shell 字符串、非法 URL、明文 secret |
| `protocol` | MCP initialize、`tools/list`、`tools/call` 请求/响应 DTO；保留未知字段以兼容协议扩展 |
| `transport` | `StdioTransport` 与 `HttpTransport`；统一读写、超时、取消和关闭语义 |
| `connection` | 单 Server 连接状态机、握手、心跳/断线通知和重连退避 |
| `manager` | 多用户/多 Server 目录、连接池、并发上限、工具缓存与调用编排 |
| `policy` | 用户/应用/模块权限、工具 allow/deny、高风险确认和网络边界 |
| `audit` | 写入脱敏使用记录与连接事件；禁止保存完整参数和结果 |

`McpManager` 不持有数据库事务跨越网络请求。配置和状态先读库，网络调用完成后以短事务写回状态/日志。

### 3.2 连接生命周期

```text
Disabled -> Configured -> Connecting -> Ready
                         |             |
                         v             v
                       Error <----- Degraded
                         ^             |
                         +-- Backoff --+

Ready/Degraded -> Disconnecting -> Configured
```

- `Disabled`：用户关闭 Server，不允许新连接或工具调用。
- `Configured`：配置有效但没有活动连接。
- `Connecting`：握手或工具发现进行中；同一 Server 只允许一个连接尝试。
- `Ready`：握手完成、能力已记录、工具清单可用。
- `Degraded`：连接曾成功但心跳/请求失败，按可恢复错误分类进入最多 3 次有界恢复。
- `Error`：配置、认证、协议或启动失败；等待用户修复或手动重试。
- `Disconnecting`：取消读写任务并等待子进程/HTTP stream 关闭。

关键分支 ID 沿用功能主文档第 7.3 节的 B03-01–B03-15（stdio、initialize、HTTP、工具发现、工具调用、连接检查、请求超时、结果解析、JSON-RPC、监控、ping、断开、重连、连接尝试、耗尽），不得将同一 ID 重新赋予配置校验或权限拒绝等其他含义。具体测试证据见功能文档第 10 节。

## 4. 数据模型与迁移边界

MCP 主表、连接事件和工作台工具引用分别由 m008、m009、m010 增量迁移管理。具体列类型和约束只在 [V1 数据模型](V1-数据模型.md) 维护；本节是逻辑摘要，不是可直接执行的 SQL。

### 4.1 `mcp_servers`

| 字段 | 说明 |
| --- | --- |
| `id` | 稳定 UUID；不使用外部 Server 名称作为主键 |
| `user_id` | 所属本地用户；所有读写必须带用户条件 |
| `name` | 用户可见名称 |
| `transport` | `stdio` / `http` |
| `config_json` | 非敏感连接配置；stdio 的 executable/args/cwd，http 的 url/allow_insecure_http；环境由宿主 allowlist 继承，不接受用户任意环境变量或认证 header，凭据由独立入口写入 OS 凭据库 |
| `credential_ref` | 可空的宿主凭据引用，不保存 secret |
| `enabled` | 是否允许连接和工具调用 |
| `auto_connect` | 启动时是否尝试连接，默认 `false` |
| `status` / `last_error_class` | 可恢复状态与脱敏错误类别 |
| `created_at` / `updated_at` / `deleted_at` | 生命周期审计 |

活动名称的唯一约束以数据模型为准；删除采用软删除，释放连接与凭据但保留任务引用和审计，不级联清除使用记录。

### 4.2 `mcp_server_tools`

保存最近一次成功 `tools/list` 的快照：`server_id`、工具名、标题、描述、输入 schema、输出 schema、schema hash、发现时间和协议版本。工具名在同一 Server 内唯一；刷新失败不覆盖上一次可用快照。

### 4.3 `mcp_usage_logs`

记录 `user_id`、`server_id`、工具名、任务/应用引用、请求 hash、结果类别、耗时、错误类别和时间。默认不保存完整参数、返回正文、密钥或模型提示词。

### 4.4 运行时数据不入库

子进程句柄、HTTP stream、取消 token、并发 semaphore 和内存中的连接对象只存在于 `McpManager`。应用重启后从 `mcp_servers` 恢复配置为 `Configured`，不假设旧连接仍存在。

应用正常退出时先停止接收新的连接、确认、刷新和调用，再取消监控与在途请求、撤销未消费确认，等待审计收尾并关闭所有会话。连接发布和调用持有运行时读锁；退出取得写锁后排空会话，避免连接完成与退出竞争留下子进程。stdio 显式终止并回收直接子进程；Streamable HTTP 对已获得的 session ID 尽力发送同源 DELETE（最多 1 秒），legacy SSE 关闭长连接。远端拒绝 DELETE 不阻止退出，也不自动重试工具。

Workbench 的未完成工具任务在正常退出时撤销提案、释放正文并终结为既有 `interrupted / process_interrupted`，不新增数据库枚举；已终结任务不可改写。MCP 已发送调用保持实际观察到的 success/error/cancelled 等类别；仅遗留 pending 在下次启动恢复为 interrupted。Tauri 退出清理整体最多等待 5 秒；强制杀进程或超时不承诺完成远端取消，仍按启动恢复处理，不重放操作。本节细化 D021/D022 的生命周期，不改变授权默认值。

### 4.5 `mcp_connection_logs`

增量 migration `m009_create_mcp_connection_logs` 保存脱敏连接事件；字段与索引只在 [V1 数据模型](V1-数据模型.md#mcp_connection_logsmcp-连接事件) 维护。m008 已应用的用户数据库通过 m009 升级，不重建 Server 或清理旧工具/使用记录。

## 5. Tauri 命令边界

具体字段、错误类别和返回 DTO 只在 [V1 接口契约](V1-接口契约.md) 维护；以下命令为职责摘要，不另行定义 LAN API：

| Command | 作用 | 关键失败 |
| --- | --- | --- |
| `list_mcp_servers` | 按当前用户列出配置和状态 | 数据库错误 |
| `get_mcp_server_config` | 获取非敏感配置用于编辑 | Server 不存在、用户越权 |
| `create_mcp_server` | 校验并保存配置，不自动连接 | 非法命令/URL、重复名称、凭据引用无效 |
| `update_mcp_server` | 更新配置；连接中变更先断开再应用 | 活动任务占用、配置非法 |
| `delete_mcp_server` | 软删除并释放连接 | 活动工具调用、服务器不存在 |
| `connect_mcp_server` | 启动/连接、initialize、tools/list | 启动失败、超时、协议错误、认证失败 |
| `disconnect_mcp_server` | 取消请求并关闭连接 | 关闭超时仍需最终标记状态 |
| `refresh_mcp_tools` | 重新发现并原子替换工具快照 | 连接不可用、工具响应非法 |
| `list_mcp_tools` | 读取最近成功的工具快照 | Server 不存在 |
| `call_mcp_tool` | 经 policy/schema/超时检查后调用工具 | 无权限、工具不存在、超时、工具返回错误 |
| `cancel_mcp_tool` | 当前用户对指定 Server/call_id 请求取消；不等待在途调用锁 | 返回 false 表示当前无对应在途调用，不宣称外部已回滚 |
| `list_mcp_usage_logs` | 查询脱敏使用记录 | 用户越权 |
| `list_mcp_connection_logs` | 查询脱敏连接事件和重试次数 | 用户越权 |
| `set_mcp_bearer_token` / `clear_mcp_bearer_token` | 独立保存/移除 OS HTTP 凭据，返回引用并断开连接 | 非 HTTP Server、OS 凭据库不可用 |

错误类别及稳定映射以 [V1 错误码](V1-错误码.md) 与接口契约为准，不由本节另起枚举。错误消息可以用户可读，但不得包含 secret、完整命令环境或完整工具参数。

## 6. 前端交互设计

### 6.1 MCP 管理页

- 列表显示名称、传输类型、启用状态、连接状态、最近错误和最后成功连接时间。
- “添加 Server”分为 Stdio/HTTP 两种表单；命令参数使用可编辑数组，不提供 shell 脚本输入框。
- “测试连接”只执行 initialize + tools/list，不会自动启用或把工具注入 Workbench。
- 工具详情显示名称、描述、输入 schema 和最近发现时间；明确“工具声明不等于已授权”。
- 删除前显示活动任务占用；禁用立即阻止新调用，等待现有调用结束后断开。

### 6.2 Workbench 集成边界

Workbench 后续只消费 `list_mcp_tools` 返回的已启用、已授权工具描述；模型提出工具调用时由宿主调用 `call_mcp_tool`，而不是让模型直接访问 MCP 连接。第一阶段不自动选择 Server，用户必须显式选择或接受应用默认绑定。

D022 细化：当前桌面消费路径必须显式选择，不使用隐式应用默认绑定；固定工具 schema hash，逐次显示参数并由用户批准。Workbench 使用 Manager 的 schema-pinned/cancellable 入口，调用前在快照锁内比较确切 schema；批准只引用宿主提案 ID，不接受模型或 renderer 替换参数。MCP 工具调用 DTO 和管理页面手动调用仍保持兼容，工具引用新增于 m010，不扩充任务状态枚举。

### 6.3 连接恢复通知与明文传输确认

- 连接列表返回仅运行时的 `reconnect_attempts` / `reconnect_exhausted`，不新增数据库状态。自动恢复停止后，在整个桌面应用的导航下方持续显示 Server 名称、停止恢复提示及管理入口；不依赖操作系统通知授权，不自动重放工具。手动连接重置预算，恢复成功或禁用/删除后提示消失；重启不恢复旧重试预算。
- HTTP 配置增加 `allow_insecure_http`，缺省 false，保存在非敏感 config_json。HTTPS 和精确 loopback 不需此标志；远程明文 HTTP 只有用户勾选风险确认后才可保存/连接。提示明确包含工具参数、结果和认证 token 可被旁路读取/篡改。修改 URL 时前端清除确认；后端新 HTTP 配置未携带 true 一律按拒绝处理。此为第 2 节既有冻结网络边界的实现细化，不扩大自动连接默认值。

### 6.4 工具结果契约

发现时同时编译 input/output schema（不启用外部 URL 解析）；schema hash 绑定两者，输出 schema 变化也使旧确认/任务失效。调用结果必须是合法 MCP content 数组，`isError` 必须为布尔值，structuredContent 若出现必须是对象。声明 outputSchema 的成功结果必须提供符合该 schema 的 structuredContent；工具主动 isError 可不满足成功输出 schema。拒绝畸形内容与 schema 不符的结果，审计为既有 error / mcp_protocol_error，返回固定脱敏错误且不自动重试，不把未验证结果继续送入模型。未知扩展字段保留，不主动获取 resource URI 或执行返回内容。

## 7. 安全、并发与可观测性

- stdio 不经过 shell；拒绝路径穿越、隐式重定向和不受控环境变量。命令启动前执行可执行文件和工作目录检查。
- HTTP 仅允许配置声明的 URL；凭据由宿主服务注入，日志只记录 credential ref 的稳定标识。
- 同一 `(user_id, server_id)` 只维护一个活动连接；工具调用使用有界并发，单次调用必须有超时和取消 token。
- 工具发现刷新与工具调用可并发，但刷新采用快照替换，不能清空上一份可用清单。
- 重连使用指数退避和最大尝试次数；收到 401/403、协议错误或用户主动断开时不自动重试。
- 连接日志记录状态转换、错误类别、耗时和重连次数；不记录完整命令行、环境变量、参数或返回正文。

## 8. 验收与测试追踪

AC01–AC15 的编号、Given/When/Then 与逐项测试资产只在 [功能主文档第 5.1 节](../../03-功能规格/V1/01-底层能力/03-MCP管理系统.md#51-ac-逐项测试设计表) 维护；本设计不复制或重新分配编号。历史矩阵与主文档不一致处，以主文档为准。

实际证据见该功能文档第 10 节：fake stdio/HTTP 集成、状态机与数据库测试、前端 mock IPC、官方 SDK 兼容 smoke，以及 `real_provider_mcp_smoke` 的真实 Provider→MCP→Provider CLI smoke。macOS 原生凭据库已单独完成隔离合成 token 的读写删除测试；这些证据仍不替代完整桌面安装验收、签名分发或 Windows/Linux 运行时门禁。

## 9. 实施顺序

1. **基础 crate**：实现协议 DTO、transport trait、状态机和内存连接池，先用 fake server 测试。
2. **持久化**：新增 migration、SeaORM entities/repositories、用户隔离和脱敏日志。
3. **Tauri 集成**：按第 5 节暴露命令，补权限检查和启动/关闭生命周期。
4. **前端管理页**：列表、表单、工具快照、连接状态和错误恢复交互。
5. **Workbench 桥接**：在 MCP Manager 单独验收通过后，加入显式工具选择和受控调用。
6. **发布门禁**：完成真实 stdio Server、真实 HTTP Server、异常/并发/重连、凭据脱敏和 E2E 验收。

## 10. 后续实现前的非阻塞确认项

1. 现有凭据服务接入时，OAuth refresh 和撤销能力需要单独扩展，但不得改变 D021 的“不保存明文 secret”约束。
2. Workbench 的用户可见摘要格式可以在 FR-006 设计中细化，但 MCP Manager 仍只保存 D021 规定的脱敏记录。
3. 真实 MCP Server 兼容性测试需要至少覆盖一个 stdio Server 和一个 HTTP Server；具体测试镜像不改变协议边界。


## 11. 实现约束与待补边界（2026-10-27）

- stdio 按 MCP 标准使用 newline-delimited JSON；独立 reader 按 request id 分发，超时移除 pending 请求，子进程随连接释放而关闭。不采用 LSP 的 Content-Length 帧。
- `McpManager::new` 默认拒绝调用；宿主通过 `McpCallPolicy` 注入 `mcp.call` 决策。Tauri MCP 命令只接受可信 main 本地页面，身份由宿主固定为 `default-user`，不接受 renderer 选择用户；这不是 FR-008/009 的完整多用户授权实现。
- 工具描述不作为可信只读声明，当前所有工具均按未知副作用处理；确认令牌在内存中保存 120 秒、一次性消费，并绑定用户、Server、工具、参数、任务与 schema hash。工具面板先审阅精确 JSON，再显式确认调用，发送过程中锁定参数并防重复提交；模型不得调用确认入口。
- HTTP URL 必须结构化解析，拒绝 URL userinfo、fragment、伪 localhost 域名和自动重定向。Streamable HTTP 按事件增量读取，不等待 SSE EOF；session header 与协商后的 `MCP-Protocol-Version` 复用。对初始化返回 400/404/405 的兼容 Server，适配器执行 legacy SSE endpoint 发现，并以 GET 长连接接收事件、POST 双通道发送 JSON-RPC；endpoint 必须保持配置 URL 的 origin。
- initialize 只接受实现已知的协议版本（当前 `2024-11-05`、`2025-03-26`、`2025-06-18`），并记录 Server capabilities/serverInfo。Server 主动 `ping` 自动响应；其他主动请求显式返回 `-32601`，不会因为 Server 描述而获得宿主权限。
- 凭据引用与普通 config_json 分离；`McpCredentialVault` 由宿主注入，桌面使用 OS 凭据库。作用域绑定用户、Server、endpoint 和引用；Bearer token 用 `Zeroizing<String>` 包装、不实现 Debug/Serialize。URL/transport 变化、轮换及删除清理旧凭据；已配置凭据无法读取时 fail-closed，不静默匿名连接。macOS 原生读写已验证；Windows/Linux 运行时门禁待补。
- 每 Server 的生命周期变更串行，在途调用结束后才允许禁用/更新/删除；全局调用并发上限 8。工具发现最多 1,000 个工具/100 页，重复 cursor 拒绝，握手与发现整体最长 30 秒。刷新在同会话调用 tools/list，发现期间可并发调用；短事务发布快照前等待在途调用结束，随后只撤销该 Server 的旧确认。失败保留完整旧快照和会话，DB 故障不会留下卡住的 connecting。
- auto_connect 失败不阻塞启动；首次失败不循环重试；只有曾成功连接的可恢复错误进入最多 3 次恢复。每 30 秒监控一次，首次符合条件的 tick 发起恢复；前两次失败后分别至少等待 30/60 秒，第 3 次失败立即耗尽，不发起第 4 次。实际时点还受 tick 粒度和连接耗时影响；手动连接重置预算，主动断开/禁用后不重连。
- auto_connect 只控制启动连接；手动建立过的连接在非主动断开后也允许恢复。心跳不仅检查 transport，还校验 JSON-RPC error，5 秒超时后进入 degraded。连接、断开、失败和重试事件写入 m009 的 `mcp_connection_logs`，只记录状态、类别、耗时和次数；页面可按 Server 查看。
- 使用记录先写 pending 再执行工具，完成后只更新结果类别和耗时；结果写库失败通过 audit_warning 提示，绝不把已经执行成功的工具伪报为可安全重试。当前用户拥有的 Server 拒绝调用写 denied；启动时将遗留 pending 转为 interrupted，不自动重放。

### 11.1 取消与中断审计实施契约

- 调用由 `(宿主用户, Server, call_id)` 标识；call_id 在审阅阶段生成并绑定确认指纹，不作为可重试的幂等键。注册表用作用域守卫清理，任务被丢弃也不残留可取消的旧句柄。
- 取消命令不等待生命周期写锁；排队/等待锁可取消。发送后取消仅停止等待并尽力发协议取消通知，不杀共享 stdio 进程、不重试、不把取消等同回滚。
- stdio 使用有界专用 writer 队列，已交给传输层的 JSON 帧由 writer 完整写入或将传输判为失败；取消/超时不会截断半条帧再把后续请求拼接进去。writer 单次写入最长 15 秒，常规取消不关闭共享会话。
- 结果与取消并发时，以已观察到的完成结果为准；如果只观察到取消/超时，则保守显示外部结果未知。晚到响应不进入其他调用的 pending 槽位。
- 拒绝审计仅写当前宿主用户拥有的 Server；参数只存 hash。pending 启动恢复为 interrupted，不改写 success/error/timeout/cancelled 等已终结记录。
- 这些契约补充既有超时/取消目标，不改变 D021 的授权与日志脱敏默认值；实现证据逐项回写功能文档，不以契约代替验收。
