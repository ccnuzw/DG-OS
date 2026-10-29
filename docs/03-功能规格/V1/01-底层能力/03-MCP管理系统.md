---
title: "MCP 管理系统"
version: V1
feature_id: V1-FR-003
domain: 底层能力
spec_status: Ready
risk: 高风险
track: MCP 能力
updated: 2026-10-29
---
# V1-FR-003：MCP 管理系统

## 1. 功能元信息

| 项目 | 内容 |
| --- | --- |
| 编号 | V1-FR-003 |
| 功能名称 | MCP 管理系统 |
| 所属版本 | V1 |
| 领域 | 底层能力 |
| 规格状态 | Ready |
| 优先级 | P0（必须） |
| 风险等级 | 高（涉及本地进程、网络凭据、工具权限与外部副作用） |
| 上游依赖 | V1-FR-001（Provider 框架，MCP 工具可能调用 AI） |
| 下游影响 | V1-FR-006（AI 工作台）、V2 画布系统 |
| 规格负责人 | TBD |
| 实现状态 | 见 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)，本文不记录 |

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
- **Skills**：以 SKILL.md 为核心的内容包，为任务提供方法、步骤和上下文；本身不授予执行权限
- **MCP**：标准工具协议，工具既可能读取数据，也可能修改文件或调用外部服务
- **权限边界**：Skill、模型和工具描述均不能授权；MCP 调用由宿主按 D021/D022 受控执行

**技术设计**：本功能的实施基线见 [V1 MCP 管理系统技术设计](../../../04-技术架构/当前版本/V1-MCP管理系统技术设计.md)，其 V1 边界已由 [D021](../../../06-决策记录/V1-冻结决策.md#d021-mcp-v1-客户端边界与安全默认值) 冻结；实现状态只在版本实现状态文档维护，具体证据与限制见本文第 10 节。

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
- ✅ MCP 非敏感配置与独立 OS 凭据引用

**不包含**（明确说明原因）：
- ❌ 实现 MCP 服务器端 → DGOS 只做客户端
- ❌ MCP 协议扩展 → 严格遵循 Anthropic 标准
- ❌ MCP 服务器开发 SDK → V8 开发者生态时提供
- ❌ MCP 服务器市场 → V3 应用市场时考虑
- ❌ 自动发现 MCP 服务器 → V2 考虑

## 3. 核心流程

### 3.1 配置和连接 MCP 服务器流程

1. 用户在可信本地主窗口新增名称、transport 与连接配置：stdio 为 executable/argv/cwd，不经过 shell；HTTP 为 URL 与独立凭据引用。
2. 非回环明文 HTTP 默认拒绝；用户另行确认风险后保存 `allow_insecure_http: true`。更换 URL 时重新确认并清理旧凭据绑定。
3. 保存配置只写数据库，不启动进程、不发网络请求；`enabled` 与 `auto_connect` 默认关闭。
4. 用户启用后主动连接：建立传输 → initialize/initialized → 分页 tools/list → 验证 schema → 事务发布工具快照 → ready。失败保留脱敏错误，不伪造成功。
5. 只有显式开启 auto_connect 的 Server 才在启动监控中尝试连接；不阻塞应用。工具不会因连接而自动进入所有任务。

### 3.2 调用 MCP 工具流程

1. 应用显式选择已连接、启用的工具；Workbench 固定 Server/工具/输入与输出 schema 联合摘要。
2. 模型只提出结构化工具调用；用户审阅确切 Server、工具及 JSON 后逐次批准。正文指令、工具描述和 Skill 内容不能授权。
3. 宿主重验 `mcp.call`、工具存在性、输入 schema、固定摘要和一次性确认。在调用路径中不会隐式重连或重放。
4. 在外部发送前保存 pending 审计；全局最多 8 个调用在途，其余有界等待并可取消。
5. 发送 tools/call；校验结果 envelope/content，以及声明 outputSchema 时的 structuredContent。协议错误脱敏并终止 Workbench；合法普通 isError 可由模型解释。
6. 只持久化引用、hash、耗时和结果类别。取消/超时提示外部结果未知；不自动重试。结果写库失败显式返回 audit_warning。

### 3.3 连接状态监控流程

1. 每 30 秒检查启用的 Server；已连接会话发送 ping，最长等待 5 秒，同时检查 JSON-RPC 错误。
2. 心跳失败后关闭旧会话，标记 degraded 并记录脱敏 disconnected 事件。
3. 下一次符合条件的监控 tick 才尝试重新连接。曾成功连接的可恢复错误最多重试 3 次；认证/配置/协议错误停止循环并要求人工处理。
4. 耗尽后标记 error，通过运行时 `reconnect_exhausted` 在全局页面提示；恢复、禁用或手动重置后提示消失。通知只查询元数据，不重放工具。

### 3.4 自动重连流程

- 监控粒度为 30 秒。首次符合条件的 tick 发起恢复；失败后按 30 秒基数指数退避，前两次失败分别至少等待 30/60 秒，再尝试下一次；第 3 次失败立即耗尽，不发起第 4 次。实际触发还受监控 tick 粒度与连接耗时影响。
- 启动 auto_connect 首次失败不无限循环；只有本进程内曾成功连接、且错误可恢复的 Server 才进入恢复预算。
- 手动断开或禁用后不自动重连；手动连接重置预算。每次恢复重新握手和发现工具，不恢复已执行/未知结果的工具调用或旧确认。
- 应用退出停止新请求，取消监控与在途操作，撤销提案/确认并释放会话；整体清理上限 5 秒。重启只恢复配置和 interrupted 审计，不重放操作。

## 4. 数据模型

### 4.1 涉及的数据表

物理列、约束、索引的唯一来源为 [V1 数据模型 §4.4](../../../04-技术架构/当前版本/V1-数据模型.md#44-mcp-管理)，不在此复制 SQL。

| 表 | 职责 / 迁移 |
| --- | --- |
| `mcp_servers` | 用户范围非敏感配置、独立 credential_ref、启用与状态摘要；m008 |
| `mcp_server_tools` | 同 Server 工具唯一、输入/输出 schema 与联合 hash 的成功快照；m008 |
| `mcp_usage_logs` | pending→终结类别、请求 hash 和任务引用；不存完整参数/结果；m008 |
| `mcp_connection_logs` | 脱敏连接/断开/重试事件；m009 |
| `workbench_task_tools` | 绑定真实任务 ID 的不可变工具引用；m010、D022 |

### 4.2 读写与恢复边界

- 所有目录、配置和日志查询按宿主用户隔离；禁用的配置仍可见，不等于允许连接。
- 生命周期变更等待在途调用，删除采用软删除并释放连接/凭据；保留任务引用和审计。
- 工具发现完整校验后以短事务替换快照；失败保留旧快照，不持有事务跨网络。
- 进程句柄、重连预算、`reconnect_attempts/reconnect_exhausted`、确认令牌和任务正文仅在内存；不是数据库列。
- 重启时 runtime status 回 configured，遗留 pending/运行任务回 interrupted；不得恢复授权或自动重放。

## 5. 验收标准

### 5.1 AC 逐项测试设计表

> 保留原有 AC01–AC15 的语义与编号。fake 证据不等同于完整 AC 或真实 Server 验收。资产路径相对 `crates/dgos-mcp`；裸函数名位于 `tests/manager.rs`。

#### AC01 配置 Stdio MCP 服务器

Given 本地用户进入可信主窗口

When 保存 executable + argv

Then 配置落库且保存不连接

#### AC02 配置 HTTP MCP 服务器

Given 本地用户进入可信主窗口

When 保存 URL 与凭据引用

Then 通过校验，credential_ref 独立保存

#### AC03 连接 Stdio MCP 服务器

Given Server 已配置并启用

When 启动连接

Then initialize/initialized 完成并进入 ready

#### AC04 连接 HTTP MCP 服务器

Given Server 已配置并启用

When 启动连接

Then HTTP initialize 成功

#### AC05 获取工具列表

Given 连接已建立

When tools/list 分页

Then 校验并缓存工具列表

#### AC06 调用 MCP 工具

Given 工具已发现且通过宿主权限

When 确认后调用指定工具

Then 返回工具结果

#### AC07 心跳检测

Given 连接已建立

When 发送 ping

Then 连接保持

#### AC08 断开检测

Given 心跳超时

When 5 秒无响应

Then 标记断开

#### AC09 自动重连

Given 曾成功连接且非手动断开

When 触发重连

Then 恢复 ready

#### AC10 重连失败处理

Given 最多 3 次重连仍失败

When 停止重连

Then 标记不可用并通知用户

#### AC11 并发调用

Given 多个应用同时调用

When 5 个并发请求

Then 所有请求正确配对并成功

#### AC12 连接日志记录

Given 连接事件发生

When 查询连接日志

Then 包含脱敏 connected/disconnected 事件

#### AC13 工具使用记录

Given 调用完成

When 查询当前用户日志

Then 仅含引用、hash、耗时和结果类别

#### AC14 MCP 服务器启用/禁用

Given Server 已配置

When 禁用 Server

Then 阻止新调用并等待在途调用结束后断开

#### AC15 刷新工具列表

Given 已有工具缓存

When 点击刷新

Then 成功替换，失败保留旧快照

#### AC 逐项测试设计

证据状态按 spec-docs-pro 词表记录：未创建 / 已创建 / 静态核验 / 本地通过 / 预发通过 / 生产通过 / 失败 / 不适用。原“证据限制”列的内容移入“备注”，不代表升格。

| AC | 验收重点 | 测试层级 | 目标资产 | 目标命令 | 证据状态 | 备注 |
| --- | --- | --- | --- | --- | --- | --- |
| AC01 | 配置 Stdio MCP 服务器 | 单元/集成 | `server_crud_isolation_defaults_and_duplicates` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 配置测试通过 |
| AC02 | 配置 HTTP MCP 服务器 | 单元/集成 | `tests/credentials.rs`、`credential_refs_are_separate_from_config_and_fail_closed_without_vault` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 作用域隔离/轮换/清理通过；macOS OS 凭据另有测试证据 |
| AC03 | 连接 Stdio MCP 服务器 | 单元/集成 | `tools_discovery_schema_confirmation_and_redacted_audit` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake stdio 通过；另有第 10.4 节官方 filesystem smoke |
| AC04 | 连接 HTTP MCP 服务器 | 单元/集成 | `src/transport_tests.rs`、`tests/credentials.rs` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake Manager 认证握手/调用与官方 SDK HTTP/SSE smoke 通过；macOS 原生 HTTP 连接见第 10.3 节，非跨平台发布验收 |
| AC05 | 获取工具列表 | 单元/集成 | `tools_discovery_schema_confirmation_and_redacted_audit` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 通过 |
| AC06 | 调用 MCP 工具 | 单元/集成 | `tools_discovery_schema_confirmation_and_redacted_audit` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake echo、官方 filesystem list_directory 与 SDK echo smoke 通过；macOS 原生 Provider→MCP→Provider 与 filesystem 调用见第 10.3 节，非跨平台发布验收 |
| AC07 | 心跳检测 | 单元/集成 | `src/manager.rs::lifecycle_tests` | `cargo test --offline -p dgos-mcp` | 已创建 | 状态机测试；真实心跳待验收 |
| AC08 | 断开检测 | 单元/集成 | `src/manager.rs::lifecycle_tests::heartbeat_rpc_error_and_timeout_disconnect_then_recover_without_startup_autoconnect` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 心跳 RPC error 和 5 秒超时通过 |
| AC09 | 自动重连 | 单元/集成 | `src/manager.rs::lifecycle_tests` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 自动成功、退避和手动断开通过 |
| AC10 | 重连失败处理 | 单元/集成 | `src/manager.rs::lifecycle_tests` | `cargo test --offline -p dgos-mcp` | 已创建 | 退避耗尽覆盖；全局恢复失败通知已有 mock UI 覆盖，真实故障恢复仍待原生验收 |
| AC11 | 并发调用 | 单元/集成 | `five_simultaneous_calls_complete_with_reverse_order_responses` | `cargo test --offline -p dgos-mcp` | 已创建 | fake barrier 验证 5 在途倒序响应；另有 128 请求突发/8 在途上限测试；真实多应用待验收 |
| AC12 | 连接日志记录 | 单元/集成 | `connection_events_are_persisted_isolated_bounded_and_redacted` | `cargo test --offline -p dgos-mcp -p dgos-db` | 本地通过 | fake 事件持久化/隔离/条数上限及 m009 存量迁移通过 |
| AC13 | 工具使用记录 | 单元/集成 | `tools_discovery_schema_confirmation_and_redacted_audit` 及第 10.2 节取消/中断专项 | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 脱敏/隔离、拒绝/取消、pending 中断恢复和结果审计失败提示通过 |
| AC14 | MCP 服务器启用/禁用 | 单元/集成 | `disabling_waits_for_inflight_call_then_revokes_connections_and_consent` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 通过 |
| AC15 | 刷新工具列表 | 单元/集成 | `refresh_uses_same_session_replaces_snapshot_and_revokes_only_its_consent`、`failed_refresh_preserves_entire_snapshot_and_live_session`、`snapshot_database_failure_rolls_back_without_stuck_connecting_status` | `cargo test --offline -p dgos-mcp` | 本地通过 | fake 同会话刷新、分页失败保留与 DB 回滚通过 |


### 5.2 边界条件测试

| 边界条件 | 测试场景 | 预期结果 |
| --- | --- | --- |
| MCP 服务器不存在 | Stdio 命令找不到 | 返回"服务器启动失败" |
| HTTP 服务器不可达 | HTTP URL 无法连接 | 返回"无法连接服务器" |
| 工具调用超时 | 工具执行超过 30 秒 | 返回超时错误 |
| 工具参数错误 | 传入无效参数 | 返回"参数校验失败" |
| 连接断开 | 网络中断或进程崩溃 | 仅对曾成功连接且非用户主动断开的 Server 做有界重连 |
| 并发调用 | 100+ 并发请求 | 正常处理，不崩溃 |

## 6. 接口设计

### 6.1 Tauri Commands（桌面端）

准确 DTO、命令参数与错误映射只在 [V1 接口契约 §7](../../../04-技术架构/当前版本/V1-接口契约.md#7-mcp-管理接口) 维护；实现入口为 `src-tauri/src/commands/mcp.rs`。

- 配置：list/get/create/update/toggle/delete；保存不连接。
- 生命周期：connect/disconnect/refresh/list tools；不接受 renderer 的 user_id，宿主派生当前用户。
- 凭据：独立 set/clear bearer token，只返回引用、不返回 secret。
- 调用：confirm → call、独立 cancel；确认绑定确切参数、call_id、task_ref 与联合 schema hash，120 秒一次性消费。
- 审计：按当前用户/Server 查询脱敏 usage/connection logs；默认 50、上限 100 条。
- Workbench 结构化提案与批准命令见接口契约 §10.0 和 D022；前端批准只提交提案 ID，不能替换参数。

### 6.2 HTTP API（局域网）

本功能的桌面 MCP Client HTTP 传输不等于 DGOS 开放局域网 HTTP API。V1 当前不把 MCP 管理命令自动暴露到网络；需 FR-008/009/010 的身份、权限和网络门禁单独验收后再开放。

## 7. 技术设计

> 实施基线见 [V1 MCP 管理系统技术设计](../../../04-技术架构/当前版本/V1-MCP管理系统技术设计.md)。早期 SQL/API/伪代码保留在[历史快照](../../../99-历史归档/V1-FR-003/2026-10-28-旧MCP流程与实现草图.md)，不继续作为实现模板；AC/B03 身份不变。

### 7.1 MCP 协议关键消息

- initialize 首选 `2025-06-18`，只接受实现已知的 `2025-06-18` / `2025-03-26` / `2024-11-05`；客户端 capabilities 为 `{}`，不宣称 roots/sampling/elicitation 支持。
- 握手后发送 notifications/initialized，Server 有 tools 能力时分页 tools/list。输入/可选输出 schema 必须为 object 且本地可编译，不抓取外部引用。
- tools/call 使用 JSON-RPC request ID 配对；支持 content 与可选 structuredContent/isError。Server 主动 ping 响应 `{}`，未知主动请求返回 `-32601`。
- stdio 为 newline JSON，不采用 Content-Length；HTTP 优先 Streamable HTTP，legacy SSE 内部兼容，保持同源与凭据作用域。

### 7.2 核心流程伪代码

以下仅表达分支与安全顺序，实际类型/错误以代码和接口契约为准。

```text
connect(config):
  if stdio: spawn executable + argv without shell            # B03-01
  else: open HTTP / same-origin legacy SSE                   # B03-03
  initialize; validate known version/capabilities; initialized # B03-02
  paginate tools/list, compile schemas, publish snapshot      # B03-04

call(explicit_request):                                      # B03-05
  require enabled + connected; never implicitly reconnect     # B03-06
  require host permission, input schema, pinned hash, consent
  persist pending intent; send with timeout/cancellation      # B03-07
  validate content + output contract; redact/audit outcome    # B03-08

send(request): correlate JSON-RPC IDs; reject invalid frames  # B03-09
monitor: every 30 seconds                                    # B03-10
  send ping, wait up to 5 seconds                            # B03-11
  on error/timeout: close old session, mark degraded           # B03-12
  schedule bounded recovery unless manually disconnected     # B03-13
  retry connection + discovery, never replay tools            # B03-14
  on exhausted budget: mark error, log + global notice         # B03-15
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

错误码与类别的唯一来源为 [V1 错误码 §MCP](../../../04-技术架构/当前版本/V1-错误码.md)，实现映射见 `crates/dgos-mcp/src/error.rs`。不得沿用历史草图中已冲突的编号含义。

配置、权限、确认、schema、协议、连接、取消和超时均返回稳定类别；错误正文不得包含凭据、完整参数或 Server 返回的任意错误内容。

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
| 凭据分离存储 | 非敏感配置进入 SQLite；认证 secret 只进入 OS 凭据库，数据库仅存 credential_ref |
| 进程隔离 | Stdio MCP 独立进程运行 |
| 敏感信息脱敏 | 日志中配置只显示摘要 |

### 9.3 可用性需求

| 需求 | 实现方式 |
| --- | --- |
| 自动重连 | 断开后自动重连 3 次 |
| 连接监控 | 心跳检测，实时状态 |
| 错误恢复 | 连接有界恢复；调用失败不自动重试或重放，取消/超时不宣称外部回滚 |

## 10. 实现与验证

> 本轮按快速迭代策略收口：下列测试用于发现明显回归和验证核心链路，不把签名、跨平台凭据、完整安装升级和全量桌面 E2E 作为当前开发阻塞；这些项目在发布阶段单独建立门禁。

### 10.1 后端测试

**目标资产**：
- `crates/dgos-mcp/src/transport_tests.rs`
- `crates/dgos-mcp/tests/manager.rs`
- `crates/dgos-mcp/tests/credentials.rs`
- `crates/dgos-mcp/tests/fixtures/mcp-server.mjs`
- `crates/dgos-mcp/tests/fixtures/mcp-http-server.mjs`
- `crates/dgos-mcp/tests/fixtures/mcp-official-sdk-http.mjs`
- `crates/dgos-mcp/tests/official_compat.rs`（默认 ignored 的官方兼容性门禁）
- `crates/dgos-db/src/mcp_tests.rs`

**验证命令**：
```bash
SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/Library/Developer/CommandLineTools/usr/bin/clang \
CC=/Library/Developer/CommandLineTools/usr/bin/clang \
AR=/Library/Developer/CommandLineTools/usr/bin/ar \
cargo test --offline -p dgos-mcp -p dgos-db
```

**验证记录（2026-10-28，arm64 linker 环境）**：
- dgos-db：3/3 通过（m008 表结构、m009 对存量 MCP 数据库增量迁移并保留数据、m010 工作台工具引用增量迁移）
- dgos-mcp 单元/transport：22/22 通过（stdio、HTTP JSON、协议版本/能力协商、Streamable HTTP/SSE、legacy SSE 双通道、主动请求响应、超时、错误脱敏、状态机、心跳失败自动恢复、取消注册表隔离/守卫清理、排队取消、大帧超时后不截断 stdio、HTTP session 关闭）
- dgos-mcp credentials：3/3 通过（token 校验、作用域隔离、认证 Streamable HTTP 与 legacy SSE 握手/调用/取消及取消通知、轮换和 URL 变化清理、DB 不含 secret）
- dgos-mcp manager：27/27 通过（用户隔离、CRUD、快照、input/output schema、确认令牌、5 并发、128 请求突发、版本协商、无 tools 能力、重连、凭据 fail-closed、使用/连接审计、同会话刷新、DB 故障回滚、取消及晚到响应、拒绝审计、中断恢复、最终审计写入失败和正常退出/握手退出竞争）
- 证据等级：fake stdio/HTTP 集成测试；另有下方官方兼容性 smoke，不将两者混算为完整 E2E。

**macOS 原生凭据验证**：同一 arm64 环境执行 `cargo test --offline -p dgos --lib os_keychain_roundtrip_for_isolated_synthetic_token -- --ignored`：1/1 通过。创建随机隔离的合成测试条目，验证写入、读取、删除和删除后不存在；不读用户既有密钥。Windows/Linux 尚未做原生运行时验收。

**全仓编译检查**：同一 arm64 环境执行 `cargo check --offline --workspace --all-targets` 通过；未覆盖跨平台链接或发布签名。

**分支追踪**：B03-01–B03-15 沿用第 7.3 节定义，不重新编号。Stdio/HTTP transport 测试覆盖 B03-01–03、B03-09；分页/刷新覆盖 B03-04；权限/确认、5 并发乱序响应和超时测试覆盖 B03-05–09；状态机测试覆盖 B03-10–15 的 fake 监控、心跳超时、自动恢复和重试耗尽路径，真实第三方故障恢复仍待验收。

### 10.2 前端集成测试

**目标资产**：
- `src/pages/McpManagement.tsx`
- `src/stores/mcpStore.ts`
- `tests/mcp-management.test.tsx`

**验证命令**：
```bash
npm test -- --run tests/mcp-management.test.tsx
```

**验证记录（2026-10-28）**：14/14 通过（全仓前端共 38/38）。覆盖所选 Server 工具/连接历史隔离、键盘操作、禁用未启用 Server 的连接、argv JSON 保留空格与中间输入、保存不自动连接、编辑点击的 Server 与 auto_connect、双步精确参数确认和防重复、关闭后恢复焦点、不自动重试工具错误/审计警告、凭据独立存储及输入清空、真正调用发现刷新命令、确认阶段取消不发送工具、在途取消绕过调用锁、取消失败仍保留运行状态和外部结果未知提示；新增远程明文 HTTP 风险确认/URL 变化清除，以及全局恢复耗尽通知。jsdom 仅在测试中补 native dialog 方法；不把 mock IPC 升格为桌面 E2E。

**取消/中断专项资产**：`src/calls.rs`、`src/transport_tests.rs::abandoned_stdio_write_does_not_truncate_frame_or_poison_next_request`、`src/manager.rs::lifecycle_tests::queued_call_can_cancel_before_dispatch_without_waiting_for_call_slots`、`tests/manager.rs::cancelled_stdio_call_keeps_other_calls_alive_and_ignores_late_response`、`tests/manager.rs::rejected_calls_are_audited_only_for_owned_servers_without_arguments`、`tests/manager.rs::abandoned_pending_call_recovers_as_interrupted_without_replaying_or_rewriting_terminal_log`、`tests/manager.rs::final_audit_failure_reports_executed_result_and_retains_unknown_pending_intent`。这些是 AC06/AC13 的异常与安全补充，不重新分配 AC/B03 ID。

### 10.3 E2E 测试

**macOS 原生 IPC 验证（2026-10-28）**：构建并运行隔离标识 `com.dgos.mcp-acceptance` 的 debug unsigned app bundle（63.97 MiB，嵌入前端 `tauri://localhost`），单独数据目录，不修改正式 Provider/MCP/任务记录。

- 通过原生界面新增并连接官方 SDK Streamable HTTP echo Server；保存不自动连接。显式选择已缓存的 `gpt-6.1-sol`、不使用 Skill、只选 echo，模型提出 `{"text":"DGOS-MCP-NATIVE-20261028"}`，界面展示精确 JSON 后单次批准，实际 MCP 结果回传模型并得到最终文本。任务 `3b212748-638b-4248-8b94-2676d120503f` 成功，596 tokens；数据库恰好一条关联任务的 success 使用记录。
- 通过原生界面新增官方 filesystem stdio Server，仅授权 `/tmp/dgos-mcp-native-data` 合成目录；发现 14 个工具。双步确认 `list_directory`，返回 `[FILE] acceptance.txt`，使用记录 #2 success，3 ms。未读写用户文件。
- 视觉检查确认工具选择、参数审查和结果文字可读；隔离验收 Provider 的凭据副本已在退出后清空并禁用，数据库压缩清理旧页，正式配置不变。
- 本次是实际 Tauri IPC、真实 Provider 和官方 Server 的 macOS debug 原生证据；不是 release/签名/安装包验收。debug 模式仍从仓库读取 bundled Skills；不以本次证据证明 release 资源路径或 Windows/Linux 运行时。退出/重启、取消与配置变更等其他原生场景仍须分别验证。

**真实 Provider tools 端到端 smoke（2026-10-28）**：新增 `crates/dgos-workbench/examples/real_provider_mcp_smoke.rs`。命令从正式数据库只读读取已启用的 `Nextcc1` Provider 和缓存的 `gpt-6.1-sol`，将配置复制到临时数据库，连接本地真实 stdio MCP fixture，执行真实 Provider 结构化 tool proposal → 单次批准 → MCP 调用 → Provider follow-up。固定无敏感提示完成，结果为 `succeeded`，3,627 tokens，恰好 1 条 success 脱敏 MCP 审计；临时数据库在进程结束后清理，正式 Provider/MCP/任务数据未写入。该证据证明真实 Provider tools 适配与 Workbench 编排链路，不替代 Tauri 桌面视觉/IPC 和跨平台发布门禁。

**证据边界**：第 10.4 节官方兼容 smoke、第 10.5 节 mock/fixture 与本节原生验证分别记录，不互相升格为完整跨平台 E2E。

**macOS release 资源/退出核对（2026-10-28）**：使用隔离标识 `com.dgos.mcp-release-acceptance` 构建 `DGOS MCP Release Acceptance.app`（release、unsigned、12.81 MiB）。原生页面成功加载并浏览 6 个 bundled Skills，证明 `resource_dir()/skills` 路径可用；通过 `super+q` 退出后以进程检查确认无残留。该包未签名、未安装，不能证明签名分发、安装升级或跨平台运行时。

### 10.4 官方兼容性 smoke

- 固定外部依赖：`@modelcontextprotocol/server-filesystem@2026.8.31`、`@modelcontextprotocol/sdk@1.32.2`，依赖安装在 `/tmp/dgos-mcp-official`，不进入项目 `package.json`。
- 命令：
  `DGOS_OFFICIAL_NODE=/Users/apple/.local/bin/node DGOS_OFFICIAL_FILESYSTEM_SERVER=/tmp/dgos-mcp-official/node_modules/@modelcontextprotocol/server-filesystem/dist/index.js DGOS_OFFICIAL_MCP_SDK_PREFIX=/tmp/dgos-mcp-official cargo test --offline -p dgos-mcp --test official_compat -- --ignored --nocapture`（使用第 10.1 节 arm64 环境）
- 结果：加入输出契约校验后复验 3/3 通过，分别覆盖官方 filesystem stdio、SDK Streamable HTTP 和 SDK legacy SSE；使用内存 SQLite、临时目录、合成 echo 文本和本机回环端口，不修改正式数据。
- 通过内容：initialize/能力协商、工具发现、工具调用、structuredContent 透传、同会话刷新及脱敏使用记录；这不是所有第三方 Server 的兼容性承诺。

### 10.5 Workbench 结构化工具消费验证（D022）

补充原 AC06/AC13 的消费桥接证据，同时追溯 FR-006 AC11/B06-09；不重新编号或改变 AC11 并发调用、AC12 连接日志的含义。设计/时序见[工作台技能消费设计](../../../04-技术架构/当前版本/V1-工作台技能消费设计.md#5-mcp-显式消费扩展d022)。

| 目标 | Given / When / Then | 层级与资产 | 命令与证据 |
| --- | --- | --- | --- |
| 显式选择与真实任务引用 | 已连接工具；选择后创建；事务绑定 schema hash，未确认前无工具使用记录 | 集成 `crates/dgos-workbench/tests/tool_flow.rs` | `cargo test --offline -p dgos-workbench`，fixture 通过 |
| 模型提案 → 确认 → 结果继续 | 模型返回结构化调用；批准宿主提案 ID；一次执行并以同 call ID 的 tool 消息继续模型 | 同上；Provider `tests/tool_chat.rs` | `cargo test --offline -p dgos-workbench -p dgos-providers`，mock HTTP + stdio echo 通过 |
| 真实 Provider tools 链路 | 已配置 Provider 返回真实结构化提案；批准后调用本地 MCP 并由同一 Provider 继续 | `crates/dgos-workbench/examples/real_provider_mcp_smoke.rs` | `DGOS_FORMAL_DB=... DGOS_REAL_PROVIDER_NAME=Nextcc1 DGOS_REAL_MODEL=gpt-6.1-sol cargo run --offline -p dgos-workbench --example real_provider_mcp_smoke`；2026-10-28 成功，临时 DB/固定提示 |
| 拒绝/漂移/并发/取消 | 等待确认或执行中；拒绝、schema 变化、重复批准或取消；不发新工具/不自动重试，保留实际调用类别 | `tool_flow.rs` 的 rejection/schema_pinning/duplicate_approval 测试 | 同上，包含在途取消与超限结果上下文终止 |
| 输出契约与结果阻断 | 只改变 outputSchema 或返回缺失/错误 structuredContent、畸形 content；旧提案失效或停止模型继续，不持久化正文 | `output_only_contract_refresh_revokes_pending_workbench_proposal`、`invalid_tool_results_stop_before_model_followup_and_keep_redacted_audit` | `cargo test --offline -p dgos-workbench --test tool_flow`，11/11 通过 |
| 脱敏与升级 | 已有任务；应用 m010 或任务完成；保留历史/状态 CHECK，引用不含正文/参数/结果 | `dgos-db/src/workbench_tool_tests.rs`、`tool_flow.rs` | `cargo test --offline -p dgos-db -p dgos-workbench`，通过 |
| 过期与预算 | 内存任务遗留/结果过大；超时或继续；终结并释放内存、拒绝而不截断 | `dgos-workbench/src/tools.rs::tests` | `cargo test --offline -p dgos-workbench`，通过 |
| 选择/确认界面 | 可用工具；手动选择并提交；展示确切 JSON、聚焦审查标题、确认只传提案 ID、拒绝/离页取消 | `tests/skills-workbench.test.tsx` | `npm test -- --run tests/skills-workbench.test.tsx`，10/10 通过；非真实 IPC |

验证命令均使用本机 arm64 环境：`SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/Library/Developer/CommandLineTools/usr/bin/clang CC=/Library/Developer/CommandLineTools/usr/bin/clang AR=/Library/Developer/CommandLineTools/usr/bin/ar`。全仓 `cargo test --offline --workspace`：101/101 通过，4 项默认 ignored；前端 `npm test`：38/38 通过。测试不接触已配置的正式 Provider、正式 MCP 配置或正式任务数据；本节 mock/fixture 不替代第 10.3 节原生 IPC 和真实 Provider 证据，也不能证明跨平台发布验收。

### 10.6 本机合成压力门禁

- 资产：`crates/dgos-mcp/tests/manager.rs::burst_of_128_calls_is_bounded_correlated_and_fully_audited`。
- 命令：第 10.1 节 arm64 环境增加 `DGOS_MCP_PERF_MIN_QPS=50`，执行 `cargo test --offline -p dgos-mcp --test manager burst_of_128 -- --nocapture`。
- 记录（2026-10-28）：1/1 通过，实测 701.7 QPS；128 个调用结果与审计逐一配对，最多 8 个在途调用。条件为本机 Node 合成 Server（5 ms 延迟）、内存 SQLite，计时包含审计写入。
- 证据边界：这是补充 AC11 的本机合成吞吐门禁，不代表第三方远程 Server、真实磁盘数据库或生产部署吞吐；全版本性能指标不据此统一标记达标。

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
| 本地用户管理（V1-FR-008） | 完整多用户门禁依赖 | 当前仅可信桌面单用户，不等同多用户授权验收 |
| SQLite 数据库 | 迁移依赖 | MCP m008/m009 与 Workbench m010，证据见第 10 节 |
| Provider 框架（V1-FR-001） | 消费桥接依赖 | D022 使用 Provider 结构化 tools/tool_calls；证据见第 10 节 |

## 12. 实现状态

只在 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md) 维护功能状态；本节不复制进度或百分比。实际测试与限制见第 10 节。

## 13. 变更记录

| 日期 | 变更内容 | 变更人 |
| --- | --- | --- |
| 2024-01-XX | 初始版本，基于 spec-docs-pro 创建 | Kiro |
| 2026-10-27 | 按 D021 完成首轮 MCP Rust/DB/Tauri/UI 落地；fake stdio/HTTP、并发、安全和前端测试通过，真实 Server/E2E 与凭据 provider 待完成 | Codex |
| 2026-10-27 | 补齐 OS 凭据接入、编辑和双步工具确认 UI、m009 连接事件、同会话刷新、5 并发及心跳恢复测试；证据和剩余差距见第 10 节与实现状态 | Codex |
| 2026-10-27 | 补齐按用户/Server/call_id 取消、专用 stdio writer、拒绝/超时/取消审计、启动 pending 中断恢复和取消 UI；不把取消声称为外部回滚 | Codex |

---

**维护约束**：冻结需求与接口遵循 D021/D022；实现状态仅维护于版本状态文档，测试证据按第 10 节分级记录。
