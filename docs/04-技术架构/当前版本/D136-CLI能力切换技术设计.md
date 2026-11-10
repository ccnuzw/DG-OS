---
title: "D136 CLI 能力切换技术设计"
version: V7
technical_task: D136-DUAL-RUNTIME
spec_status: Draft
updated: 2026-11-10
---
# D136 CLI 能力切换技术设计

> 风险 L3。依据已冻结 D136，本文记录 Provider-only 运行、旧混合历史切换、租约失效及清理契约；工程细节沿后续章节的实际 API 收口。代码和验证状态以唯一实现状态为准，不以本文草案标记或早期拟实施措辞推断未实现，也不提升 V7 门禁。

来源：[D136 / D135](../../06-决策记录/V7-冻结决策.md#d136-双运行时分工与统一能力管理) · [D144 输入、信任与用量增量](../../06-决策记录/V7-冻结决策.md#d144-cli-agent-直接配置精确工作区自动信任与用量交互) · [实施计划 §12](../../02-产品与版本/后续版本/V7-编程项目工作台方案草案.md#12-双运行时与统一能力管理实施计划) · [旧阶段4契约 §9](../../02-产品与版本/后续版本/V7-项目任务基础技术契约.md#9-阶段4增量契约项目级-provider-skills-mcp) · [D130 / D131](../../06-决策记录/V2-冻结决策.md#d130-单任务与cli-agent停止并等待终态) · [原生适配](../../02-产品与版本/后续版本/V7-AI-Agent-CLI适配设计.md)。实现与验证证据仍以 [V1 实现状态](../../02-产品与版本/当前版本/V1-实现状态.md) 为唯一维护位置。

## 1. 基准、授权与边界

任务包 `.herdr-team/tasks/D136-P0/B.json`，任务 `D136-P0-B`；初稿任务 base 与实际 HEAD 均为 `51c49fd7f1173209992f6160c52ba8035dc94068`。共享工作目录已有修改；源码结论来自读取的工作区内容，并非声称所有源码等同该提交。初稿仅新建本文；本轮按 `review-B.md` 仅修订本文，其他文件只读，不运行检查、测试、构建、格式化、提交或外部调用。

| specRef | 快照与当前源文件核对后的 SHA-256 |
| --- | --- |
| `V7-冻结决策.md` / `spec-0.md` | `49b2e840ac9f821b41f97ada727833e884a93fe24775993ad090ebf4a4b2e22f` |
| `V7-编程项目工作台方案草案.md` / `spec-1.md` | `57a52a1201976208385d1beebdafda9175f62c6e22309e41a19948670632c012` |
| `V7-项目任务基础技术契约.md` / `spec-2.md` | `5f269748212c1405259b727746adc9b22632723f2a1d9f57d18d004709515baf` |

P2 目标仅为共享 DG-OS Provider、CLI 原生 Skills/MCP、旧混合能力干净退出。平台 AI 服务属于 P1；原生配置文件适配、OAuth、技能安装属于 P3/P4，本文规定接口边界，不实现第二套配置写入器。下一轮须由协调者重新固定代码基准与允许路径；不得用本文的静态盘点代替实际源码快照。

## 2. 现状与风险对策

| 已读取的实现事实 | 风险 | P2 处理 |
| --- | --- | --- |
| `capabilities.rs::prepare` 在无修订时构造 `ProjectProvider::Native`；Manager 服务缺失时允许默认空 launch | 新入口遗漏 Provider 可落入原生账号 | Provider 引用必填；服务未装配、缺省绑定及旧原生会话拒绝；Provider 自身修订 0 合法 |
| `CapabilityLaunch.managed` 同时驱动 Provider、网关和两家 MCP 隔离；正修订即启动网关，包括 MCP 空选集 | 只删 Skills/MCP 选择器仍保留混合通道 | 删除 managed、网关和技能材料，建立只含 Provider 的 launch |
| Codex `managed_mcp_config` 禁用原生 MCP、`register_skills` 添加私有根；Claude `strictMcpConfig`、本地插件和 `setMcpServers` 注入网关 | 仅共享 Provider 时仍破坏原生能力 | 删除这些生产覆盖，保留原生设置、事件及审批 |
| `ProjectCapabilities::revoke` 递增 epoch 并撤销 binding/pending，不关闭 CLI | 已发出的 API key 仍在 CLI 内存或环境 | 持久撤权先阻断准入，再取消匹配的进程级租约并等待退出 |
| `ProviderManager::resolve_cli_provider_launch` 固定 owner/model/revision，读 vault 后再次检查；返回值不是可撤销租约 | 解析后到 spawn、后台原生子任务期间存在撤权窗口 | Provider 变更与 CLI 注册共享准入协调；绑定活动进程，提交前后重验 |
| `save_session` 与 `save_session_capabilities` 分两次提交；能力服务为可选 | 失败留下无绑定会话 | 创建意图与 Provider 绑定原子保存，启动失败/未知保持可核实状态 |
| `handoffs.rs` 拒绝同引擎；生成摘要会拒绝旧 session epoch | 旧混合会话无法同引擎新建交接；撤权后连只读摘要也不能生成 | 允许旧历史作为交接来源；目标重新授权，源权限不得作为读取历史的授权条件 |
| migration 000029 的 unlink trigger 删除 session 能力快照，项目 FK 级联删除修订 | 旧历史在解除关联后丢失来源 | 追加历史归档，不把存量快照改成新运行绑定 |
| `cli.rs::probe` 已有 5 秒/4 KiB/进程组清理；`prepare` 另行使用 10 秒 `output()` 后检查长度 | 重复探测且后一条并非读取时有界 | 统一复用现有探测实现，明确用户触发和能力证明等级 |

## 3. 目标运行契约

新会话只有一种运行方式：`provider = DG-OS 精确引用`，`skills = CLI native`，`mcp = CLI native`。没有 `native` Provider 选项、环境缺失回退、旧 managed 开关或长期双模式。`provider_revision` 引用 Provider 的 `config_revision`，合法范围为 `>= 0`；新建 Provider 初值就是 0。项目配置 `revision` 为 `> 0`；项目 CAS 的 `expected_revision=0` 仅表示尚无 head。旧 session 快照的项目 `revision=0` 表示历史原生模式，不能与 Provider 自身修订 0 混淆。CLI 自己的指令、Hooks、工具和权限仍由原生运行时决定；宿主只对自身准入、会话归属及生命周期负责。

Provider 专用服务可在现有 `capabilities.rs` 原位替换并改名为 `CliProviders`，不再依赖 SkillsManager/McpManager/技能发布目录。只读旧 JSON 解析类型单独命名 `LegacyCapabilitySnapshot`，不能实现 launch/authorize 或被新启动接口接受。两家各有 `ProviderLaunch` 应用器，共用 Provider 引用和进程绑定；不统一原生 RPC、审批或事件。

### 3.1 DTO

| 拟议类型 | 字段与约束 |
| --- | --- |
| `CliProviderRef` | `provider_id, model_id, provider_revision: i64 >= 0`；精确引用 `Provider.config_revision`；owner 由宿主派生；无 `source=native` |
| `CliProviderConfiguration` | `engine, provider: CliProviderRef`；`deny_unknown_fields`；不接受 skills/mcp/任意 argv/env/endpoint/凭据 |
| `CliProviderRevision` | `revision > 0, permission_epoch >= 0, configuration, created_at`；CAS 与 Provider 自身修订是不同字段 |
| `CliVersionEvidence` | `engine, normalized_version, executable_fingerprint, adapter_revision, bridge_revision?`；指纹绑定解析后的文件身份及内容摘要，不信任仅版本字符串；更换可执行文件使证据失效 |
| `CliProviderSnapshot` | `schema_version=1, revision > 0, permission_epoch >= 0, engine, provider, cli_evidence, compatibility_status=unverified/verified/failed, evidence_ref?`；兼容证据仅陈述覆盖事实，不作为逐账户付费准入凭证；不保存 secret、vault 引用或含秘密配置 |
| `CliCapabilityObservation` | `skills_source=native, mcp_source=native, observed_at?, observation_status=unreadable/not_observed/observed, native_details?`；配置快照与实际观测分开；native_details 仅白名单脱敏字段 |
| `SessionExecutionPolicy` | `legacy_read_only` 或 `provider_bound`；历史缺失/损坏/未来版本不得默认成可运行 |
| `ProviderCleanupReceipt` | `binding/session IDs, cleanup_state=not_required/pending/stopped/unknown, reason, recovery_action?, boot_proof?`；撤权与清理状态分开，外部结果仍由原 run 的 unknown 表示；不把凭据库删除成功等同 CLI 停止 |

`SessionRecord`、`OwnedSessionView`、前端 `DevelopmentSession/AgentSession` 暴露运行策略和可显示的 Provider 快照。`RunRecord` 继续使用既有 `run_id` 与 `source=native_agent`，新旧快照用显式 tagged history DTO 展示；原生 MCP 事件不能成为 DG-OS MCP usage log 或宿主执行证据。

执行会话的 `OwnedSessionView/AgentSession` 顶层另含必填 `generation: string`（宿主 UUID），每次创建或显式恢复生成新值；该值仅标识当前进程代次，不是授权凭证，也不写回普通历史。`send_agent_message` 输入为 `{ session_id, generation, request_id, text }`，`respond_agent_request` 为 `{ session_id, generation, request_id, response }`。宿主沿现有归属、Provider 租约、项目及目录身份检查，并在派发前比较当前代次；旧代次返回 `stale_generation`，不派发原生 IO。快照同时捕获 backend/代次，返回前重验，避免恢复期间拼接新旧投影。

Codex 通知按原生 thread/turn 与宿主 run 关联；早于 turn/start 回执的通知有界暂存，确认 turn ID 后再消费。审批向界面暴露不透明 UUID，内部保留原生 RPC ID、kind 和原始参数；重复请求不替换参数，冲突标 unknown，响应在 IO 前保留已派发事实。Claude 使用宿主 request UUID 作为 SDK prompt UUID，历史/实时完整消息按原生 UUID 去重，终态必须关联当前用户回合；未知身份保持 unknown，不以相同正文去重。Claude 审批继续使用 SDK requestId，派发前从可操作列表移除；失联不重新呈现可重复批准的请求。

显式恢复不发送提示词。Codex 必须返回原 thread、有效 cwd 和合法 turns/items 历史；Claude transcript 缺失或为空时拒绝恢复，不调用 query 创建替代会话；两家返回错误仍保留原有持久历史。CLI/bridge 版本和协议能力仍按 §5 边界判断，成功模型闭环证据只在实现状态文档记录。

Codex `rust-v0.167.0` 恢复按固定版本 [thread 协议](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/app-server-protocol/src/protocol/v2/thread.rs) 读取 `initialTurnsPage`、`turnsBackwardsCursor`、`itemsBackwardsCursor` 及后续 `nextCursor/backwardsCursor`。请求明确排序方向，turn 内 items 保持时间顺序；以 `(turnId,itemId)` 去重，不以正文去重。summary/notLoaded 可按原 turn 补读 full items。共享上限为64个分页RPC、每页请求128条、10秒、2000个显示事件与16 MiB内容；空页可继续，循环cursor、格式/读取失败和预算耗尽保留已取得投影并标 `history_truncated`。原生恢复成功与历史展示完整是两个结论；不创建第二历史存储、不启动替代thread。hydration结束后再次核对lease和backend状态，关闭或失效后的迟到结果不能发布为新generation。

Codex `item/permissions/requestApproval` 沿上述原生请求归属规则，独立于命令、文件、问题及MCP elicitation。依据固定版本 [permissions 协议](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/app-server-protocol/src/protocol/v2/permissions.rs)，界面展示 thread/turn/item、cwd、reason和请求的network/fileSystem权限；响应仅接受原请求的完整 `permissions` 与显式 `scope:'turn'|'session'`，或空 `permissions:{},scope:'turn'` 拒绝。不支持编辑权限、永久授权、`strictAutoReview`选项或未知路径种类；不将权限扩展当作原生MCP逐工具许可。请求结束/原生resolved、冲突、关闭、失联和generation失效均禁止迟到批准；派发前消耗请求，失败/unknown不重放。界面unknown不保留可批准入口，取消回合仍使用既有interrupt，不伪造permissions协议不存在的cancel决策。

当前产品发送入口仍是显式文本 `{text}`，没有“选择此原生Skill”的结构化输入。固定版本 [turn 协议](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/app-server-protocol/src/protocol/v2/turn.rs) 支持 `UserInput::Skill{name,path}`，但这既不是独立activation回执，也不证明正文或资源已使用。本次不新增选择器、不把清单自动加入每轮、不解析文本伪造结构化选择；管理视图只读观测不改变发送。验收继续区分配置提交、清单发现、输入选择、正文/资源读取、工具实际执行及历史恢复；不存在独立activation回执时单独标unsupported，不将虚构回执作为必需条件。Stage4的Claude Skill工具请求仅作归属观察；正文/资源读取、原生MCP逐工具许可、dispatch和结果回送等既定证据仍必须满足，不以口头声明、目录可见或预写receipt替代；缺逐工具许可仍blocked。

### 3.2 宿主 API 与迁移清单

沿用 main/Workbench/TasksRun、MaintenanceGate admission、SessionResolver owner/lease；账户 token 继续用既有 `agent_lease` 命名，不与 P1 发起全仓重命名。响应前重验 lease。读取旧历史仍校验账户归属，不接受 renderer 自报身份。

| 当前命令或方法 | P2 目标 |
| --- | --- |
| `read/save/revoke_development_capabilities` | 删除旧命令及导出；替换为 `read/save/revoke_development_cli_provider`；save 使用 project_id/expected_revision/configuration，revoke 返回 epoch 与 cleanup receipts |
| `read_development_capability_catalog` | 替换为 `read_development_cli_provider_catalog(project_id, engine)`；返回当前 owner 最小 Provider/模型目录、修订、兼容状态与固定原因，不加载 DG-OS Skills/MCP |
| `get_development_session_capabilities` | 替换为 `read_development_session_configuration(session_id)`；返回 tagged legacy history 或 Provider 快照及独立 native observations |
| `list/respond_development_capability_confirmation` | 删除；不返回永远空数组作 shim；CLI 工具批准只经 `respond_agent_request` |
| `open_agent_session` | 输入改为 tagged `create` 与 `resume`；create 必有精确 `provider_revision >= 0` 与 task_id；resume 只含 DG-OS session_id、trust_workspace，不接受替代 Provider 或任意 native_id。D144 新入口传 false，由宿主消费精确活动工作区的有效 agent 授权，不再要求重复确认；授权动作见[接口契约 §17.11](V1-接口契约.md#1711-d143-cli-agent-工作现场目录授权与退出) |
| `AgentManager::open/open_with_capabilities` | 合并为唯一 Provider-bound 创建/恢复入口；无可选 service、默认 launch 或绕过 binding 的公开路径；测试 fixture 显式构造测试专用服务 |
| `send_agent_message/respond_agent_request` | 除旧归属与 pending 校验，重验运行策略、进程级 Provider 租约、项目 epoch 及 Provider 可用性；历史发送/批准统一拒绝 |
| `prepare/accept_development_handoff` | 保留命令，扩展同引擎迁移；目标显式选择 Provider 自身修订（允许 0）及正项目配置修订；摘要只作为草稿，单独显式发送 |
| `get_agent_session` / workspace/session 列表 | 历史返回保存的有限投影及只读标记，无 backend 也可查；读取不启动 CLI |
| `read_development_cli_cutover` / 新 `recheck_development_cli_cleanup` | 前者只读返回独立 data_state、local_cleanup_state、CLI 准入及恢复动作；后者按当前 boot identity/自有 child 退出事实重新核实并持久收敛，不发送信号给历史 PID、不重发外部请求；均只展示当前 owner 对象，其他 owner 的全局阻塞仅给固定原因 |

这里是拟议 Tauri 契约，不新增 HTTP 端点；OpenAPI 无新增 HTTP 操作。正式实现须由唯一契约写入者同步直接受影响接口文档、host commands 注册、契约检查规则、DTO、调用方及中英文错误文案，不能先上线一半新旧接口。

## 4. 两家 Provider 应用与原生账号回退阻断

### 4.1 Codex

复用 `resolve_cli_provider_launch`，仅支持 adapter 明确实现的 Responses/Bearer 组合；不要求为每个 owner/Provider/model 先运行付费验证。可信子进程环境设置专用 key；process config 显式给出受控 Provider id、base_url、`wire_api=responses`、`requires_openai_auth=false` 和专用 env_key；thread/start 与 thread/resume 均显式指定 `modelProvider` 与所选模型。该逻辑不再受 managed 判断保护。

删除技能根与 MCP server 覆盖，不改原生 plugins/skills/hooks/mcp_servers。使用该版本可返回的有效 thread/config 元数据核对实际 Provider/model；缺失所需可观测字段或优先级无法证明时拒绝 `provider_binding_unconfirmed`，不凭启动参数声明成功。Provider 配置项中潜在原生 auth helper/替代认证路径只能按已验证版本显式清除或覆盖；无法可靠压过项目/用户设置的版本不准入。是否存在及具体字段拼写必须在该版本协议核实，不能猜字段写入。

子进程继承环境前剔除可影响 Provider 选择的已识别原生凭据/路由变量，专用 key 无值即拒绝。读取原生 MCP 所需环境仍遵循其规则；不以清空所有环境破坏原生工具。已保存的登录账号即使有效，也不得在所选服务 401/400/断连时被使用。实现验收须组合有效原生账号与指定服务不可用，观测指定端点请求与失败终态；优先使用真实 CLI 加本地故障端点，不要求用户首配先进行付费多轮测试。实际供应商调用另行授权，证据不足如实记录。

### 4.2 Claude / bridge

继续使用 `pathToClaudeCodeExecutable`、`settingSources=['user','project','local']`、原生 canUseTool、系统提示 preset；删除 DG-OS 本地插件、skipMcpDiscovery、strictMcpConfig、dgos_project setMcpServers/reconnect 及网关 ready 前置条件。原生插件/MCP 的认证、连接和审批仍由 CLI 自己负责。

桥接 `open` 改为必需的 Provider 私有控制数据，与旧 capabilities 对象彻底分离。仅接受宿主固定 key 白名单，禁止现有宽泛 `startsWith('ANTHROPIC_')` 输入规则。同时覆盖 base URL、主模型、辅助模型别名，清空未选认证槽，禁用或拒绝 Bedrock/Vertex/Foundry/原生 OAuth、apiKeyHelper 等可改变来源的有效路由。具体字段及优先级按版本验证；不能覆盖的 native 配置产生明确不支持原因，不能永久禁用全部原生设置规避问题。

沿用 SDK 私有 stdio `applyFlagSettings({env})` 传递需要 flag-tier 优先级的 secret，不用会把 key 放入 argv 的 options.settings。初始化完成、Provider 覆盖确认前禁止 `send` 入队；桥接尚未握手确认时的 ready 事件不能使宿主状态变 idle。检验初始化本身是否可能发模型请求；若该版本无法在首次网络请求前固定 Provider，则该版本不支持 D136。所有桥接异常使用固定安全分类，不能在删除 managed 后恢复输出原始 error.message。

Claude 的必需路由控制与可选能力分开判断：启动仍须成功执行 Provider flag-tier 覆盖和有效设置读回，不因未知版本跳过核验；SDK 0.3.317 的只读 commands/models/MCP 按返回 schema 验证。未核实 CLI 版本可以尝试只读观察，但不开放可选写控制；2.1.319 作为已核对写协议组合，逐项声明 reload/reconnect/toggle 能力。确定的未知方法仅禁用对应能力，不影响普通消息；鉴权、策略、格式、超时或传输失败不冒充方法不支持。

两家均不修改用户/项目的持久 Provider 配置或登录状态；信任 CLI 包括其本地 Shell/Hooks 可能读取进程环境。无法声称 key 永远只在 Rust 内存。禁止前端、argv、数据库、日志、仓库和交接摘要携带 key；使用秘密容器，在 spawn/私有控制传输后尽快释放宿主副本，不增加无必要 String 克隆。

## 5. CLI 版本与兼容证据

只读扫描不执行程序。显式版本探测不隐式安装依赖、不运行项目脚本、不调用模型；活动草稿自动准备及继续时恢复由 D143/D144 授权规则驱动，不再要求独立启动确认。CLI 加载原生配置、Hooks/MCP 的范围在项目权限说明中可查。复用 `cli.rs::probe` 的固定 argv、无 stdin、5 秒、4 KiB 读取上限及独立进程组清理，删除 capabilities.rs 的第二条 10 秒探测。探测返回原有 not_installed/not_executable/start_failed/timeout/output_limit/protocol_unsupported 分类。

| 具体组合 | 依据与证据等级 | 下一轮处理 |
| --- | --- | --- |
| Codex 0.167.0 / App Server | 实施 C 观察到的本机版本；不是本轮探测或 D136 通过证据 | 以新路径复验 adapter/schema、有效路由确认、原生能力共存及无账号回退 |
| Claude Code 2.1.319 / Agent SDK 0.3.317 | 实施 C 观察到的 CLI 版本；SDK 候选为 0.3.317；不是 D136 通过证据 | 复验初始化前后路由、辅助模型、SDK 私有控制及原生能力 |
| Codex 0.166.0、Claude 2.1.318 / SDK 0.3.317 | 阶段4历史 Provider 证据；Claude 2.1.317 另有 D127 原生证据 | 仅历史参考，不称为当前版本，不直接提升为 D136 新路径通过 |
| 其他版本、预发布、未知 App Server/SDK schema | 本轮未探测 | 按 adapter 对已识别协议/schema 的支持判定；不能仅用 `>=最低版本` 推断支持，无法识别或已知不兼容则拒绝 |

生产准入只有实现必需的硬边界：受支持 adapter/protocol/auth、当前 owner 的 Provider 启用且凭据/模型可用、精确 `config_revision >= 0`、当前 lease/epoch，以及初始化阶段在模型请求前固定路由并确认实际 Provider/model。已知不兼容或不能证明无原生账号回退的实现路径拒绝。`protocol_verified=false` 是 ProviderManager 未完成引擎实测的事实，不等于不兼容；不得无条件改成 true，也不得单凭 false 阻断可支持的普通创建/恢复。

兼容证据可为 `unverified/verified/failed`，可记录 CLI 文件身份/版本、adapter/bridge、protocol/auth、覆盖项及无秘密证据引用。证据可缺省、过期或未验证；不伪称通过，也不因没有某 owner/provider revision/model 的真实付费证据拒绝运行。不新建兼容性准入表、隔离验证模式或用户验证产品流程。已知失败须区分确定不支持（拒绝）与历史凭据/网络故障（按当前可用性处理），不能把一次网络失败永久变成不兼容。

实现阶段以真实 CLI + 本地 HTTP/MCP fixture 证明路由、错误、防回退及生命周期；真实供应商多轮、流式、工具和恢复证据须另行授权，缺失时明确标为未覆盖。用户首配新 Provider（`config_revision=0`）可直接显式创建普通会话；实际模型调用按该动作授权，不额外强迫先跑多轮工具验证。失败/未知不自动重试；CLI 原生重试能力按版本记录。创建/恢复重新检查二进制文件身份，变化时重探测；文件一致性不足不宣称不可篡改。

## 6. 数据模型与一次性迁移

### 6.1 复用结构与最小 DDL

追加 migration，序号由集成者分配；不修改 000026～000030。复用 `development_capability_revisions/heads/session_capabilities/run_capabilities`。配置与快照追加 `format`，旧数据默认 `legacy_mixed_v1`、新数据为 `cli_provider_v1`，新旧 JSON 独立解析，旧字节不改。heads 仍引用原修订表；切换事务清空旧 head revision、epoch 加一。新保存的项目 revision 按历史最大值+1 追加，CAS expected_revision=0 仅表示无当前 head，不重用旧主键。旧执行 API 删除，底层事务和表复用；run 不新增台账。

只新增 session binding（合并撤权/清理）与必要 legacy 历史归档；不新增平行 Provider revisions/heads/run、独立 revocation cleanup 或 compatibility 表。最小 DDL 如下；新列默认只分类旧数据，不授予权限。

```sql
ALTER TABLE development_capability_revisions ADD COLUMN format TEXT NOT NULL
  DEFAULT 'legacy_mixed_v1' CHECK(format IN ('legacy_mixed_v1','cli_provider_v1'));
ALTER TABLE development_session_capabilities ADD COLUMN format TEXT NOT NULL
  DEFAULT 'legacy_mixed_v1' CHECK(format IN ('legacy_mixed_v1','cli_provider_v1'));
ALTER TABLE development_run_capabilities ADD COLUMN format TEXT NOT NULL
  DEFAULT 'legacy_mixed_v1' CHECK(format IN ('legacy_mixed_v1','cli_provider_v1'));
ALTER TABLE development_migrations ADD COLUMN cleanup_boot_identity TEXT;
CREATE TABLE development_cli_session_bindings(
  owner TEXT NOT NULL CHECK(length(owner)>0), session_id TEXT NOT NULL,
  execution_policy TEXT NOT NULL CHECK(execution_policy IN ('legacy_read_only','provider_bound')),
  project_id TEXT, task_id TEXT, revision INTEGER,
  permission_epoch INTEGER NOT NULL CHECK(typeof(permission_epoch)='integer' AND permission_epoch>=0),
  provider_id TEXT, model_id TEXT, provider_revision INTEGER,
  state TEXT NOT NULL CHECK(state IN ('opening','active','closed','unknown','revoked')),
  cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('not_required','pending','stopped','unknown')),
  cleanup_reason TEXT, host_instance_id TEXT, boot_identity TEXT,
  cleanup_proof TEXT CHECK(cleanup_proof IS NULL OR cleanup_proof IN ('owned_exit','boot_changed','no_execution')),
  request_digest TEXT, updated_at BIGINT NOT NULL,
  PRIMARY KEY(owner,session_id),
  FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner) ON DELETE CASCADE,
  CHECK((execution_policy='legacy_read_only' AND revision IS NULL
      AND provider_id IS NULL AND model_id IS NULL AND provider_revision IS NULL
      AND state IN ('closed','unknown','revoked'))
    OR (execution_policy='provider_bound' AND project_id IS NOT NULL AND task_id IS NOT NULL
      AND typeof(revision)='integer' AND revision>0
      AND provider_id IS NOT NULL AND length(provider_id)>0
      AND model_id IS NOT NULL AND length(model_id)>0
      AND typeof(provider_revision)='integer' AND provider_revision>=0))
);
CREATE INDEX development_cli_provider_cleanup
  ON development_cli_session_bindings(owner,provider_id,provider_revision,state);
CREATE TABLE development_cli_legacy_history(
  owner TEXT NOT NULL, session_id TEXT NOT NULL,
  original_project_id TEXT, original_task_id TEXT,
  session_record TEXT NOT NULL CHECK(json_valid(session_record)
    AND length(CAST(session_record AS BLOB))<=1048576),
  capability_snapshot TEXT CHECK(capability_snapshot IS NULL OR length(CAST(capability_snapshot AS BLOB))<=1048576),
  referenced_configuration TEXT CHECK(referenced_configuration IS NULL OR length(CAST(referenced_configuration AS BLOB))<=65536),
  original_revision INTEGER, original_permission_epoch INTEGER, archived_at BIGINT NOT NULL,
  PRIMARY KEY(owner,session_id),
  FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner) ON DELETE CASCADE
);
```

`session_record` 是旧索引白名单投影（engine/native_id/目录/项目任务来源），不复制消息全集。capability_snapshot/referenced_configuration 保留原字节，损坏历史也原样归档，不以 decode 失败丢弃。归档在 unlink 之前完成且不可覆盖；每个旧 session 仅内嵌其引用的配置，无第二套 legacy revisions 表。其他旧修订留原表；升级前已丢来源明确标缺失，不虚构还原。

追加 migration 还须安装归属/状态触发器：provider_bound 的 opening/active 必须匹配同 owner/session/project/task，revision 指向同项目的 `format=cli_provider_v1`，JSON engine/provider 与 binding 完全相等；新格式 session/run 快照须匹配既有归属及新 binding。legacy 永不可更新为 provider_bound；revoked 永不可恢复，epoch 不倒退。正常 closed 的新模式会话仅在 cleanup 已收敛、原 Provider/epoch 仍有效并重新授权后可转 opening 恢复；unknown 不直接恢复。保留 000029 原触发器，新运行 unlink/项目删除先同事务撤权和保留来源，再解除关联；opening/active 不得直接脱离。binding 不设会级联毁掉历史的 project/task FK，活动态由上述触发器约束，session 复合 FK 始终隔离 owner。

Provider 引用不 FK 到可删除 providers 行；运行前由 ProviderManager 重验。配置/快照保持 64 KiB/1 MiB 上限，新增分类和身份文本由 repository 固定或有界输入生成，无秘密。新增两张 owner 表纳入 `user_data_repository.rs` Workbench 清理顺序和 orphan 清理；账户删除仍按现有策略。CLI 证据/实际 Provider 快照继续放复用的 session/run snapshot，不在 binding 重复存 JSON。

### 6.2 数据迁移、旧进程清理与 unknown 分离

`InstanceLock` 防同数据目录双宿主，不能单独证明旧后代消失。数据事务、本地清理事实、外部结果 unknown 独立。普通已关闭历史、旧消息 unknown 或仅缺少新 boot 字段，不得推断为存在活进程，不新增整机重启或全局 CLI 准入门槛。只有已知自有活进程、明确未完成的回收，或有实际证据的跨宿主未清理对象进入 cleanup 流程；限制作用于有证据的对象/目录，不把普通历史升级成全局阻塞。

| 条件 | 可执行动作 | 收敛条件 |
| --- | --- | --- |
| 本宿主仍有 child/进程组句柄 | 独立确认停止，按 D130 停并等待终态；取消不发停止 | wait/可追踪后代回收确认后记 owned_exit；ack、旧 PID 不在或 UI 关闭不足 |
| 数据待迁移 | 关闭 CLI 新准入，事务归档全部旧历史、创建 legacy binding、清空旧 heads 并加 epoch，最后写 `d136-cli-provider-v1` marker | 跨宿主 cleanup unknown 也可提交数据迁移；失败全回滚，不回开旧网关 |
| 有证据的跨宿主未清理对象 | 优先按可靠进程身份（启动身份/创建时间/可追踪进程组）和退出证据核实；不能只凭旧 PID | 无法可靠核实才提供保存 boot 基线后系统重启的最后恢复选项；不将普通历史一并冻结 |
| 数据 complete，无真实未清理阻塞 | 允许新 Provider 会话；旧 session 只读 | 历史外部 unknown 保留，不重放、不恢复旧授权 |

boot identity 仅是有证据未清理对象的最后恢复证明，不是每次迁移的强制字段。平台 adapter 若使用 boot 证明，须来自可信 OS、同机器同启动稳定、重启改变；应用 UUID/墙钟/用户勾选不能替代。机器差异不算重启，字段缺失不能凭空创建活进程风险。

首次旧数据迁移：普通已关闭历史照常归档，缺 boot 不阻止新 CLI；未完成 run 保持历史 unknown，也不单独代表当前有进程。已知自有 child 必须停等终态；真实清理失败或跨宿主未清理证据保存对象归属与具体原因。可追踪对象用可靠身份和退出事实收敛；不可追踪且无法核实的对象才保存当前 boot 基线并提示系统重启后重新核实。首次 boot 读取失败仅影响选择该证明路径的对象，不能将整个应用永久冻结。

重核实只能收敛本地 cleanup，不能把外部 unknown 改成功、恢复旧会话或按旧 PID 发送信号。新 boot 可证明旧本地进程不存在，外部网络/远端任务不在证明范围。具体查询/停止/退出证据入口由下一轮 Manager 对齐实际可追踪能力；本 Provider 切片不实现系统进程探测。其他 owner 对象只返回固定原因，普通历史不得引入跨账户全局重启要求。

宿主显式重核实入口为 `recheck_development_cli_cleanup(app, window) -> Result<(), String>`：复用 Workbench/TasksRun 与当前账户 lease，维护准入后调用 `CliProviders::recheck_cleanup`，返回前重验 lease；不接受前端提供 machine/boot/owner，不返回全库清理数量。仅同机器且不同启动标识可收敛既有 pending/unknown 清理，不发送信号、不重放启动或恢复旧授权。无法读取可信启动标识返回 `boot_identity_unavailable`；工作台错误旁提供重核实操作，检查完成不等于目录已可用。

所有既有 session 均为 legacy_read_only：D135 混合、旧项目修订0原生、无快照索引及待归类历史；只有 Provider 的旧快照也不复用 native_id。损坏历史保留原字节，不能完整归档则回滚；旧令牌/pending 不迁移。历史只读保存索引、run 和配置，不 thread/resume/Claude query；注明有界投影和缺失原生消息，不伪造完整历史。

### 6.3 新建交接

复用 `development_handoffs` 的持久 reservation、fingerprint、可编辑 summary 与目标新 UUID，不复制旧批准、令牌或 Skills/MCP。允许同引擎新建，拒绝把它称作恢复。读取源历史仍查 owner，但不要求旧 epoch 等于当前授权；生成目标时重新绑定当前项目/task/epoch，并核对目录身份与当前文件指纹。

待归类历史须先显式关联到自有、同规范目录的项目任务，保持历史标记与旧来源；不得修改历史快照使其变成可执行。摘要必须明确新建会话与未知/未验证事项，沿用脱敏与 64 KiB 上限；accept 不自动发送摘要。目标失败或未知保留原 reservation，用原 ID 只读核实；不能再次生成另一个目标进程规避失败。账户重新登录仍不能恢复旧租约。

## 7. 单元契约、并发与事务

| 拟议单元 | 前置条件 | 返回与副作用 | 幂等 |
| --- | --- | --- | --- |
| `CliProviders::save` | owner/project、合法 engine/引用、expected_revision | 追加新修订并 CAS head，不停止活动会话 | stale CAS 拒绝，客户端未知时先读 head |
| `AgentManager::create_provider_session` | 数据 complete、本地 clear、当前 lease/目录信任、项目 revision>0、Provider config_revision>=0、受支持适配与当前可用性 | 记录 opening 绑定，事务外启动并确认路由；可保留兼容 unverified；成功 active，失败清理记终态 | session UUID+输入摘要；opening/unknown 只核实，不重复 spawn |
| `resume_provider_session` | 非 legacy、原 native_id、原 Provider/CLI 证据仍有效 | 显式恢复原新模式会话，无配置替换，无自动模型消息 | 原 UUID，关闭/未知按生命周期核实 |
| `authorize_provider_run/respond` | 绑定 active、epoch/revision/lease 均有效 | 校验后写 run 快照或原生批准；失效先撤销进程租约 | 复用原 run/request receipt，不重放 |
| `revoke_project_cli_provider` | owner/project，归档项目也能撤销 | epoch 单调增加、所有匹配 session 置 revoked、cleanup 意图同事务；事务外取消并关闭 | 反复撤销可继续提高 epoch；同 cleanup 对象不重复 spawn/副作用 |
| `ProviderManager` 配置失效通知与进程绑定协调 | 所有 update/delete/toggle/credential migration 成功提交 | 按 owner/provider 失效对应旧绑定，取消自有 CLI，保存清理事实；普通名称/排序变化不热改活动快照 | 不撤销新版本绑定；持久 revision/cleanup 可重新核实 |
| `migrate_cli_provider_v1` | 实例锁、CLI 准入关闭；有自有 child 必须先停等终态，跨宿主未知可单独记录 | 一次事务归档、分类、清空旧 head/递增 epoch、记录 boot 基线、最后写数据 marker | 重复不归档/加 epoch；清理与外部结果独立收敛 |

P2-PROVIDER 实际锁序：Manager admission（宿主已有时）→ 共享 `credentials_lock`（异步提交/spawn gate）→ 短 binding registry mutex。所有 ProviderManager clone 共用二者；最终同步 spawn 与变更提交不能穿透 gate。网络模型刷新在锁外；vault 解析沿既有凭据锁，不能在 spawn closure 等待网络、握手、DB或异步任务。提交后仍在锁内同步取消旧租约，释放锁后才调用清理 sink；sink 不得 panic、重入 manager 或同步等待 child。未安装 sink 时 CLI lease 注册拒绝，普通 AI Provider 操作不依赖该 sink。

创建期间先登记 opening 的进程级 child lease，再解析 Provider；变更并发发生时可取消尚未进入 active 的握手。spawn 前、握手后写 active 前以及每次 send/respond 前重验 Provider 修订和 lease；撤权后晚到 ready 不得登记。进程令牌独立于账户 token，撤销某项目/Provider 不取消同账户其他运行。原生后台工具未必经过宿主 send，故失效通知必须直接取消进程，不能只等下一次 begin_run。

普通项目默认配置保存和 Provider 非安全元数据保存不擅自重启运行；活动绑定固定旧配置，后续宿主准入按 revision 变化拒绝且提示新建。禁用、删除、凭据/endpoint/认证来源变化属于立即失效并停止的路径；重新启用不复活旧绑定。Provider 变更已提交但进程清理失败，不回滚配置或谎报整个保存失败：配置结果与独立 cleanup receipt 各自呈现。

SQLite 使用现有事务/先取得写锁再 CAS 模式，无网络、vault 或进程等待在事务中。新 session 索引、Provider opening binding 及输入摘要同事务；握手结果、native_id 与 active 状态同事务。失败不删除已开始原生历史；DB commit 失败立即取消 launch lease、清理自有进程，并保留可核实记录。run 与 Provider 快照同事务，避免存在 running run 而缺少授权绑定。

### 7.1 DB 可独立实施接口

P2-DB 已写入以下 repository 接口源码；本批按协调者要求不运行测试、构建或格式化，验证状态由集中执行结果决定。新增类型从 `dgos_db::repositories::development_repository` 导出，复用 `StoredAgentSession`、`StoredRun` 与 `StoredCapabilityRevision`。方法均为 async，无外部进程、vault 或模型调用；事务方法不能拆成多个提交。唯一追加 migration 为 `m20261107_000032_cli_provider_cutover`，000031 及更早 migration 保持原样。

```rust
migrate_cli_provider_v1(ctx: &CutoverContext) -> Result<CutoverReceipt, String>;
save_cli_provider_revision(owner: &str, project_id: &str, expected_revision: i64,
    configuration: &CliProviderConfiguration) -> Result<StoredCapabilityRevision, String>;
cli_provider_revision(owner: &str, project_id: &str, revision: Option<i64>)
    -> Result<Option<StoredCapabilityRevision>, String>;
reserve_cli_session(owner: &str, session: &StoredAgentSession,
    opening: &OpeningBinding, snapshot: &CliProviderSnapshot) -> Result<OpeningReceipt, String>;
activate_cli_session(owner: &str, session_id: &str, binding_id: &str,
    host_instance_id: &str, native_id: &str, expected_epoch: i64) -> Result<StoredCliBinding, String>;
begin_cli_run(run: &StoredRun, snapshot: &CliProviderSnapshot) -> Result<(), String>;
revoke_cli_binding(target: &CleanupTarget) -> Result<bool, String>;
revoke_cli_bindings(owner: &str, scope: &CliRevocationScope) -> Result<Vec<CleanupTarget>, String>;
record_cli_cleanup(owner: &str, session_id: &str, expected_binding_id: &str,
    expected_host_instance_id: &str, observation: &CleanupObservation) -> Result<CleanupReceipt, String>;
recheck_cli_cleanup(proof: &HostBootObservation) -> Result<u64, String>;
read_cli_cutover(owner: &str) -> Result<OwnedCutoverView, String>;
read_cli_session_configuration(owner: &str, session_id: &str) -> Result<SessionConfiguration, String>;
```

`CliProviderConfiguration` 实际字段为 `engine/provider_id/model_id/provider_revision`，拒绝未知字段；Provider 修订允许 0。`CliProviderSnapshot` 在这些字段上增加 `binding_id/revision/permission_epoch` 与有界 evidence JSON；全部只保存无秘密审计数据。`OpeningBinding` 另含 project/task、host/可选 boot、request_digest。reserve 原子写 session、binding 和新格式 session snapshot；恢复只接受已收敛的 closed、新 binding_id、原 native_id/目录/可执行文件/模型/Provider 引用/项目修订/epoch，不替换原配置。activate 按 owner/session/binding/host/epoch CAS，重验 head，已有 native_id 必须与握手结果一致，再原子保存 native_id 与 active。旧 session upsert 不得替换已绑定身份。begin_cli_run 原子写既有 run、不可修改的快照与任务状态；cutover marker 后旧 `start_run` 拒绝全部 native run，独立 host run 路径保留。`cli_provider_revision` 读取当前或指定 Provider 修订，旧 `capability_revision` 仅读取 legacy 格式。

`revoke_cli_binding` 是 Provider sink 的精确入口：直接消费通知捕获的 owner/session/binding_id/host，返回是否匹配当前 opening/active；迟到通知不得按 provider/revision 范围重新扫描。`revoke_cli_bindings` 的 Project scope 提高 epoch、清空 head，并返回活动 cleanup targets；Provider scope 仅用于显式整 Provider 撤销，同时撤销 closed 可恢复绑定并保留其已完成 cleanup，不用于模型级或旧版本通知。record 按 binding_id+host CAS，正常退出转 closed；revoked 保持 revoked，仅收敛 cleanup，unknown 可后续核实。recheck 返回受影响行数，只对已有 pending/unknown、保存了不同可信 boot 基线的终态绑定收敛本地清理，不恢复授权、不改外部 run unknown；输入必须来自宿主。

迁移事务归档原始历史字节、使旧 head 失效并递增 epoch，最后写 marker；format 由 DDL 默认分类，不通过更新损坏历史 JSON 完成分类。普通历史及未完成 run 不单独产生本地 cleanup blocker；`CutoverContext.owned_exit_sessions` 只记录宿主已确认退出的对象，实际 child 停等仍是宿主前置责任。`read_cli_cutover` 仅返回当前 owner 的 complete/local_cleanup_pending；`read_cli_session_configuration` 返回 binding、新格式快照和历史索引投影，完整旧快照/引用配置留在归档表。unlink 在删除项目来源前归档并撤销活动绑定；新表已加入账户 Workbench purge 与 orphan 统计。

专用 `cli_cutover_tests.rs` 覆盖升级/重复迁移、坏 JSON、事务回滚、跨归属、Provider 修订0、历史最大修订追加、迟到退出与精确撤权、epoch/终态不可复活、boot 收敛、unlink 和账户清理，以及旧入口绕过、恢复身份替换、closed 撤权和 Provider head 重读。验证证据集中记录于 [V1 实现状态](../../02-产品与版本/当前版本/V1-实现状态.md)。既有 `save_capability_revision/revoke_project_capabilities/save_session_capabilities/save_run_capabilities` 暂为未切换的旧外层消费者保留，cutover marker 后拒绝写入；下一批 agents/host 切换后删除旧公开路径，不能作为新格式兼容入口。真实宿主/进程与 Provider sink 接线不由 DB 测试证明。

### 7.2 Provider 变更通知可独立实施接口

实际导出定义在 `crates/dgos-providers/src/cli_bindings.rs`（作为 manager 子模块），由 crate lib 重导出，不依赖 agents：

- `CliProviderBinding { binding_id, owner, session_id, host_instance_id, provider_id, model_id, provider_revision }`；宿主派生身份，每次进程 opening/resume 生成新 binding_id，Provider修订允许0。
- `ProviderManager::install_cli_change_sink(Arc<dyn CliProviderChangeSink>) -> DgosResult<()>` 一次安装，clone共享；`CliProviderChangeSink::changed(&CliProviderChange, &[CliProviderBinding])` 锁外收到变更和当时捕获的精确失效目标，只调度宿主持久化/回收，不做重入或wait。
- `register_cli_opening(binding, &parent_token).await -> DgosResult<CliProviderLease>` 在凭据锁内校验精确引用并登记 child token；未安装sink拒绝。`lease.binding()/cancellation_token()/is_cancelled()/cancel()` 供宿主跟踪；取消不反向取消账户parent。
- `resolve_cli_provider_for_lease(&lease).await -> DgosResult<CliProviderLaunch>` 在vault解析前后校验租约；原 `resolve_cli_provider_launch` 保留为精确引用、无网络的秘密快照解析，不构成进程准入凭证。
- `with_cli_spawn(&lease, &launch, synchronous_spawn).await -> DgosResult<T>` 在同一提交锁内重验owner/model/revision与租约，再同步创建并登记child；同一lease仅允许一次spawn尝试（失败也不自动再试），每次恢复生成新binding_id。closure不可返回延迟spawn的future。握手在锁外，宿主立即监听token直到真实exit；令牌失效时不得仅移除UI或丢弃lease。
- `validate_cli_lease(&lease).await -> DgosResult<()>` 用于握手ready及每轮send/respond前；拒绝跨manager registry/已取消/旧修订/模型失效。

`CliProviderChange { owner, provider_id, previous_revision, current_revision: Option<i64>, kind }` 的 kind 为 MetadataChanged、ExecutionInvalidated 或 CommitUncertain。相关 update/delete/toggle、手动模型增删、模型刷新、凭据迁移和排序由 manager 共享的 `ProviderMutations/TaskTracker` 持有；调用方丢弃等待 future 不会 abort 已接收的任务，也不会重复 mutation。任务持维护 ActivityPermit，取得 credentials_lock 后完成仓库提交、结果核实、同步取消及精确目标捕获，释放锁后执行 sink；清理通知不是调用方等待链上的可取消尾段。普通名称/排序不取消进程；名称修订改变后旧引用不能继续宿主准入。重新启用不复活旧lease。凭据保存等确定失败不发成功通知；DatabaseError 在锁内重读：确认未变不取消，已变或不可核实则保守取消并标 CommitUncertain，原 mutation 错误仍返回，不伪成功、不自动重试。成功提交后的核实不可读也标 CommitUncertain。sink只调度精确binding_id/session/host目标，不按revision范围重扫撤销新lease；DB持久化/真实进程退出仍由宿主消费。

执行失效分类复用 `validate_provider_config` 的有效配置：比较归一化端点、协议、实际认证方式、HTTP策略、版本头及实际发送的workspace；重试参数、无关JSON元数据及显式填写同一认证默认值均不取消进程。手动模型移除与模型刷新提交后，在同一凭据锁内核实模型缓存，只捕获所选模型已不存在或不再为文本生成的绑定；保留未受影响的手动及远程模型进程。修订仍用于后续精确准入，已有快照不热改；提交结果不明或核实失败仍按CommitUncertain保守撤销，通知只含捕获的精确binding目标。

宿主接入顺序：装sink → 持久opening意图 → 注册lease → resolve_for_lease → with_cli_spawn同步spawn并登记自有child → 锁外握手监听cancel → validate后登记active；账户/项目撤权调用lease.cancel并wait。退出runtime/关闭DB前必须 await `ProviderManager::shutdown_provider_mutations() -> DgosResult<()>`：永久关闭相关变更准入并等待全部已接收任务含锁外通知；等待者被drop不取消任务，下次调用继续drain。维护通过共享ActivityPermit拒绝与未完成提交尾段交错。tracker/关闭状态由manager clone共享，复用tokio-util rt能力，不新增总线/依赖/表；宿主shutdown接线不在本写入范围，未接线前不得宣称全生命周期集成完成。进程退出后才释放lease；所有调用使用同一manager及clone，独立manager/其他进程改库不共享即时通知。

返工新增 `mutation_tests.rs`：测试专用 post-commit barrier 在仓库成功返回且lease取消前暂停；独立连接确认DB变化、断言旧lease仍活，再abort外层等待者、释放barrier，断言token取消、精确sink和shutdown drain完成。覆盖toggle/update/delete/remove_manual_model，不用sleep碰时间窗口；测试维护permit阻止中途VACUUM、shutdown关闭准入；另含真实 `/bin/sh` fixture child 经取消终止并wait、确定回滚不取消/不发成功、未知提交保守取消且CommitUncertain。barrier仅cfg(test)，不改变生产契约。它确定性暴露“提交后、取消前”窗口；本轮按repair-B禁止中途执行，未声称实际运行过失败前/通过后或真实子进程测试。集中复验由协调者冻结本批次后执行；既有cli_binding_tests/cli_launch_tests继续覆盖其他租约边界，不调用正式vault或真实模型。

## 8. 伪代码

分支前缀 B136B 为本文稳定技术分支标识，不复用产品 AC 编号。

```text
FUNCTION cutover(instance_lock, confirmed_stop):
  close_cli_admission()                                         // B136B-01
  IF owned_child_exists: stop_and_wait_or_return_cleanup_pending() // B136B-02
  boot = trusted_host_boot_observation_or_unavailable()
  BEGIN SQLITE TRANSACTION
    IF marker_exists: validate_archives_without_reclassification() // B136B-03
    ELSE:
      validate_schema_and_lossless_archivability_or_rollback()   // B136B-04
      archive_old_sessions_with_referenced_configuration_bytes() // B136B-05
      clear_old_heads_increment_epoch_once_classify_legacy()
      persist_cleanup_unknown_and_boot_baseline_insert_marker_last() // B136B-06
  COMMIT
  recheck_cleanup_from_owned_exit_or_changed_boot_only()
  open_new_admission_only_if_data_complete_and_local_clear()

FUNCTION open_or_resume(host_context, input):
  REQUIRE current_owner_lease_directory_and_cutover_ready()      // B136B-07
  IF legacy_session_or_missing_execution_binding: RETURN legacy_session_read_only
                                                               // B136B-08
  binding = exact_provider_config_revision_gte_zero_reject_missing_native()
                                                               // B136B-09
  cli = bounded_explicit_probe_and_fingerprint()
  REQUIRE supported_adapter_protocol_auth_and_current_availability(cli)
  record_compatibility_unverified_without_paid_admission_gate()  // B136B-10
  BEGIN SQLITE TRANSACTION
    reserve_opening_binding_or_return_original_receipt()         // B136B-11
  COMMIT
  register_opening_child_lease(); resolve_secret_exactly()
  REQUIRE lease_epoch_revision_still_current()                  // B136B-12
  spawn_with_provider_only_material_no_skills_or_gateway()       // B136B-13
  IF route_not_confirmed_or_handshake_fails:
    cancel_and_reap(); persist_failure_or_unknown(); RETURN      // B136B-14
  REQUIRE lease_epoch_revision_still_current()
  BEGIN SQLITE TRANSACTION
    save_native_id_and_active_binding_or_fail_and_reap()         // B136B-15
  COMMIT
  RETURN snapshot_without_secret

FUNCTION send_or_respond(context, original_ids):
  REQUIRE owner_lease_binding_epoch_provider_pending_current()  // B136B-16
  persist_native_run_and_provider_snapshot_atomically()
  dispatch_once(); unknown_keeps_original_receipt()              // B136B-17

FUNCTION revoke_or_provider_invalidated(scope):
  BEGIN SQLITE TRANSACTION
    persist_revocation_and_cleanup_intents()                    // B136B-18
  COMMIT
  cancel_all_matching_opening_and_active_child_leases()
  close_owned_backends_concurrently_with_bound()
  persist_stopped_or_cleanup_pending_without_regrant()           // B136B-19

FUNCTION continue_legacy(context, source, target_provider):
  read_owned_history_without_spawning_or_authorizing_source()    // B136B-20
  REQUIRE current_target_epoch_directory_fingerprint_and_trust()
  reserve_new_uuid_even_same_engine(); open_new_provider_session()
  RETURN reviewed_summary_draft_without_send_or_old_approvals   // B136B-21

FUNCTION startup_or_account_end_or_exit(event):
  revoke_leases_before_cleanup(); never_replay_messages_tools()  // B136B-22
  recover_metadata_unknown_only(); do_not_kill_from_saved_pid()  // B136B-23
  IF startup: recheck_cleanup_from_trusted_boot_change_without_resolving_external_unknown()
```

## 9. 错误、外部调用与撤权生命周期

沿现有 Agent 字符串分类，不在本文分配数字错误码。保留可直接复用的 `session_invalid/session_not_found/task_not_found/project_archived/workspace_untrusted/workspace_busy/storage_failed/capability_conflict/capability_revoked/cli_version_changed/result_unknown/cleanup_pending`。新名称为拟议，正式写入错误权威文档后才实施。

| 分类 | 条件与用户后续动作 |
| --- | --- |
| `provider_required` | 缺省或 native；显式选择 DG-OS Provider；Provider 自身修订0合法，负值为非法输入 |
| `legacy_session_read_only` | 任意切换前会话执行/恢复/批准；提供新建交接 |
| `cli_cutover_pending` | 数据迁移未完成或旧本地 cleanup 未收敛；显示重试迁移/停止自有 child/保存 boot 基线后系统重启再核实等具体动作，不回开旧模式 |
| `cli_version_unsupported` | adapter/schema 不支持或已知不兼容；显式探测/选择受支持版本；没有兼容实测证据仅显示 unverified，不设拒绝错误或付费验证入口 |
| `provider_incompatible` | 协议/认证/辅助模型/运行时组合不能支持；选择兼容项 |
| `provider_revision_changed` | 旧 Provider 修订不再可用；新选择、新会话，不替换旧配置 |
| `provider_unavailable` / `credential_unavailable` | 禁用/删除/模型缺失或 vault 固定分类；不显示 secret、vault 错误正文 |
| `provider_binding_unconfirmed` | 实际 Provider 来源无法确认、原生路由冲突或初始化覆盖失败；停止启动对象 |
| `cleanup_pending` / `result_unknown` | 前者为本地清理待核实，走 §6.2 恢复出口；后者保留原外部请求 unknown，不自动重发，也不单独阻断新 CLI |
| `stale_generation` | 发送或审批来自旧进程代次；重新选择当前会话；该请求未派发原生 IO |

不再把所有 Provider DgosError 压成 provider_unavailable；固定映射保留修订、凭据、协议区别。未知内部错误统一固定安全分类，bridge/native output 经现有脱敏白名单处理；不把 endpoint、环境、完整提示词、工具参数、secret 写普通日志。

| 动作 | 等待与范围 |
| --- | --- |
| 显式关闭或迁移停止 | 原生 close 可先礼貌请求；Claude 沿用最多2秒协议关闭，再进入 Transport 的4秒回收；收到 ack 不代表 stopped；同时对所有匹配 lease 发取消再并行等待 |
| Provider 禁用/撤销 | 提交并阻断后立即取消匹配 child lease，不等原生协议或运行结束；单个 Transport TERM 等1秒、必要时 KILL 与 wait 最多2秒，4秒回收上限；目标批次并行4秒后留下 cleanup_pending |
| 账户结束 | D131 先撤销旧账户 lease，模块/聊天/任务/CLI/MCP 等并行清理；认证失败不触发；不让本切片改变其他域等待预算 |
| 应用退出 | 沿既有全局 stop、连接中 admission 最多8秒、run watcher 每项最多15秒；不声称它们等于整个退出固定5秒。现有其他 manager 的5秒 timeout 与 Agent 清理是 join 的独立分支 |
| 启动恢复 | 只恢复元数据；旧 opening/active 与未完成 run 标 unknown，按可信 boot 变化收敛本地 cleanup，外部结果保持 unknown；不重放/发模型请求/按持久 PID 发信号 |

上述是可测设计与已读源码上限，非本轮测量。停止只覆盖自有 child/进程组及可追踪后代，不停止外部 CLI/共享 MCP 服务。主动脱离的本地后代按 §6.2 系统重启证明消失，已发送网络与外部副作用不能据此撤回。Provider key 真正作废仍需供应商撤销/轮换。清理失败保留 owner、对象/目录占用与可执行恢复动作；收敛本地风险后释放占用，旧外部 unknown 永久如实保留而非永久阻塞应用。

## 10. 准确删除与迁移调用清单

本清单基于本轮工作区读取/目标搜索；无 LSP 配置，未把文本搜索当类型化引用完备性证明。下一轮导出符号重命名须在可用 LSP 下重新查引用。仓库共享修改继续发生时由集成者重新核实清单。

| 文件与当前符号/路径 | 具体动作 |
| --- | --- |
| `crates/dgos-agents/src/capabilities.rs` | 删除 `ProjectProvider::Native`、PinnedSkill、SelectedMcpTool、publish_skills、managed、skill_root/claude_plugin/mcp_url/mcp_bearer；ProjectCapabilities 替换 Provider-only 服务；旧结构只留独立只读历史解析，不留执行别名 |
| `crates/dgos-agents/src/capabilities/gateway.rs` | 删除整个 CLI 宿主网关、Binding、确认注册/响应/令牌生成与 tools/call 转发生产实现；DG-OS 平台 McpManager 不受影响 |
| `codex_session.rs` | open_with_capabilities 改 Provider-only；删除 register_skills、wait_managed_mcp、managed_mcp_config/ready、mcp_name 及专属隔离事件；保留原生 MCP 审批/事件 |
| `claude_session.rs`、`tools/agent-bridge/worker.ts` | 删除 capabilities schema、selectedSkills 插件扫描、strictMcpConfig、动态网关注册/reconnect/必须 connected；Provider私有控制单独必填；保留原生工具许可、部分消息、终态、interrupt/close |
| `manager.rs` | install_capabilities/capabilities/open/open_with_capabilities/prepare 调用统一迁移；删除可选服务和默认空 launch；snapshot 兼容历史只读展示；close/cancel_user/shutdown 改操作 Provider 进程绑定，不再操作网关 binding |
| `executions.rs` | begin_run/end_run 网关会话逻辑改为 Provider run 准入和原子快照；list_development_runs 解析新旧 tagged snapshots；watch 清理只释放当前run资源，不恢复授权 |
| `handoffs.rs` 及 `handoffs/tests.rs` | 移除同引擎拒绝与源旧epoch执行授权检查；目标 provider_revision 必填；扩展旧历史来源，保留指纹、reservation、未知不重试 |
| `takeover.rs`、`manager.rs::close_takeover_session` | 旧 capability_epoch 查询改当前 Provider epoch；停止对象/账户/目录集合与版本重验保留 |
| `crates/dgos-db/src/repositories/development_repository/capabilities.rs` | 删除旧混合执行 API，原位复用修订/head/CAS/快照表与事务工具，按 format 分离只读旧 JSON 和新 Provider 写入口；对外提供 §7.1 原子方法 |
| `crates/dgos-db/src/migrations/mod.rs`、追加 migration、repository 模块、`user_data_repository.rs` | 追加格式列、合并清理的 binding 与必要历史归档；数据 marker 和 boot 基线；不修改旧 migration，不建平行修订/兼容准入表；保留 unlink 来源及账户清理 |
| `crates/dgos-providers/src/cli_launch.rs`、`manager.rs`、凭据变更路径 | 保留精确凭据解析，增加可靠的 Provider 失效/进程准入协调；变更分类不靠 Tauri UI 单独发通知 |
| `src-tauri/src/lib.rs`、`commands/development_capabilities.rs`、`commands/agents.rs`、commands 导出 | Provider-only 生产装配、必填 DTO、全量替换旧命令注册；删除网关确认命令与 capability_admission_regression 中 Skills 锁假设 |
| `src/stores/developmentCapabilities.ts`、`agentRunnerStore.ts`、`developmentStore.ts`、`developmentDelivery.ts` | Provider-only DTO 与 history policy；移除 native Provider 默认，允许精确 Provider config_revision=0，项目 revision>0；保持账户 agent_lease/epoch/generation 命名、原UUID核实与草稿 |
| `ProjectCapabilities.tsx`、`CapabilitySnapshotView.tsx`、`TaskLaunch.tsx`、`DeliveryPanel.tsx`、`ProjectReview.tsx`、`AgentWorkbench.tsx`、`AgentConversation.tsx` | 迁移 Provider选择/实际快照；历史禁发与新建交接；删除 DG-OS Skills/MCP 注入选集与 host confirmations 轮询/计数；输入区原生管理入口归 P3/P4/P5 |
| `src/components/development/HostCapabilityConfirmations.tsx` | 删除 CLI 专属 host 确认组件/hook；其他平台 AI 宿主审批不删除 |
| 中英文 `capabilities.ts`、`delivery.ts` 及相关错误资源 | 删除仍被UI作为新运行目标的旧模式文案，保留显式历史来源描述；补 Provider错误、生效和清理状态 |
| `scripts/check-host-contracts.mjs`、相关ACL/commands导出 | 删除旧命令参数规则，登记新接口；无无用命令和空实现兼容层 |
| `crates/dgos-agents/Cargo.toml` | 生产移除仅混合注入所需 dgos-skills/dgos-mcp/axum/subtle；仍需的 HTTP 错误fixture依赖放 dev-dependencies，其他依赖按真实调用核实 |

验证资产也必须切换，不能靠保留旧API继续通过：

| 当前资产 | 下一轮去向 |
| --- | --- |
| `capabilities/tests.rs` 中网关鉴权、宿主审批、Skill发布、managed MCP隔离测试；Codex managed fixture测试 | 删除已被取代契约用例，替换 Provider固定来源、撤权停止和历史拒绝行为；平台 MCP 原有安全测试保留 |
| `crates/dgos-db/.../capability_tests.rs` | 保留旧数据升级fixture及历史读取需要的行为，旧生产写API测试改为 migration/只读证明；新增跨账户、事务回滚、unlink保留来源和账户清理边界 |
| `examples/stage4_acceptance.rs`、`scripts/stage4-acceptance.ts`、`src-tauri/src/smoke/agents_stage4.rs` | 删除旧混合验收场景/路由，保留可复用原生工具/流式/恢复驱动并迁入新 Provider-only 场景；历史证据记录不改写 |
| `examples/stage4_provider_errors.rs`、`scripts/stage4-provider-errors.ts` | 迁移真实CLI本地401/400/网络/凭据修订/版本变化用例，删除Skill缺失前置条件；补存在原生登录账号时不得fallback |
| `examples/stage5_acceptance.rs`、`smoke/agents_launch.rs`、`smoke/agents_stage6.rs` | 构造器与能力admission场景迁移至Provider服务；Git/任务/worktree原行为不缩减 |
| `tests/agent-workbench.test.tsx`、`development-review-boundaries.test.tsx` | 删除仅针对旧host确认的用例；保留原生审批归属/迟到响应/账户重置，新增legacy只读与Provider显式选择边界 |
| `tests/agent-close-boundaries.test.tsx`、`agent-output-references.test.tsx`、`development-delivery.test.tsx`、`development-drafts.test.tsx` | 删除旧确认 mock，Provider fixture 包含合法 config_revision=0，项目新修订>0；保留关闭失败、草稿、原请求核实及输出引用行为 |
| `tests/fixtures/workflow-round3-application.tsx` 与 `tools/ui-preview` 相关fixture | 移除混合host确认场景/旧目录DTO，改原生审批与历史只读/Provider失效；不改变已接受的无关视觉基线 |

Stage4 的 CLI 驱动使用 `NativeExtensions` 安装隔离 ProjectShared Skill 包和 MCP stdio 配置，不创建 DG-OS MCP Manager 执行副本。正文与引用资源 marker 留在原生包，模型用户消息只指定技能名、当前 challenge 和输出文件；工具 fresh receipt 必须同时出现在原生 MCP 结果和项目文件。记录分别覆盖清单/目录、原生读取/激活、精确工具许可、实际执行和结果回送；缺失任何必要观察则 blocked/unsupported，不能由 shell/file 许可或初始化成功补足。`--resume` 仅接受新版 artifact，核对原 native identity 与 Provider snapshot，不新建替代会话。`--desktop` 保留生产 UI/Provider 控制验收范围，不单独证明 Skill 正文或原生 MCP 获准执行。


## 11. 分支到测试追踪

本任务包没有 featureIds/acceptanceIds；下表追踪 D136 与实施计划 §12.3/12.4，不虚构正式 AC。全部为下一轮验证目标，本轮未执行。

| 分支 | 可观测验证目标 | 目标资产 / 验证层 |
| --- | --- | --- |
| B136B-01～02 | 自有活动 child 必须停止等终态，取消不停止；跨宿主 cleanup unknown 不阻止数据事务提交 | shutdown/recovery 及真实进程停止流程 |
| B136B-03～06 | 空库/旧项目修订0/损坏原字节/事务中断；格式隔离、head清空/epoch仅增一次、新revision不重用；unlink来源、owner隔离；普通已关闭历史缺boot不阻塞CLI | DB capability/user_scope 与独立进程升级fixture |
| B136B-07～09 | 缺省/native/负Provider修订拒绝；新Provider config_revision=0直接创建，项目新revision>0；legacy执行拒绝 | Agent/Provider边界、IPC和表单 |
| B136B-10 | 5秒/4KiB探测、wrapper清理、文件变化、SDK不支持；支持路径兼容unverified可正常创建，无逐账户付费验证门槛；已知不兼容拒绝 | cli真实进程fixture、Provider兼容回归；供应商实测另证 |
| B136B-11～12 | 重复UUID、opening注销、解析vault期间轮换、spawn前撤权；无迟到活动登记/旧secret准入 | `shutdown_tests.rs`、Provider `cli_launch_tests.rs` 与进程门闩 |
| B136B-13～14 | 两家原生Skill+资源/MCP可用，不启动宿主网关、不禁用原生server；有效原生账号存在时选定端点401/400/断连仍失败 | 真实CLI+隔离本地HTTP/MCP fixture；服务商多轮/流式/工具另行授权实测 |
| B136B-15 | 原生已启动但DB提交失败，进程被取消并保留失败/未知；无无绑定可运行session | DB提交故障+真实子进程+重启检查 |
| B136B-16～17 | 正常保存不改活动快照；每轮/批准重验；失效不fallback；unknown原ID不重发；原生事件不写DGOS MCP执行 | Agent run/审批行为回归、两家真实持续会话 |
| B136B-18～19 | 禁用/删除/凭据轮换/撤权并发；opening与active都取消，其他owner/provider保留；回收失败持久可查，再启用不复活 | Provider、DB、shutdown联合故障fixture及真实双CLI进程观测 |
| B136B-20～21 | 同/跨引擎新建、旧epoch源历史可读；目标重新信任和Provider选择；新UUID/native_id，summary不自动发送、无批准复制 | handoff回归、真实UI/IPC与新会话观测 |
| B136B-22～23 | 账户先失效；历史unknown/缺boot不推断活进程；自有child停等终态，真实未清理对象按可靠身份与退出证据核实，无法核实才提供boot重启出口；不按旧PID杀进程，不增全局门槛 | close/session/shutdown 与可靠进程身份fixture；系统重启证明仅按需预约 |

最低命令目标在对应代码批次完成后由协调者集中执行：受影响 `cargo test --offline -p dgos-db -p dgos-agents -p dgos-providers`；前端按上述真实改动文件定向 Vitest；`npm run contracts:check`；bridge 构建与前端 `npm run build`；真实宿主使用现有 `webview_smoke_agents` 目标。新增 smoke phase 名由实现确定，不能预写为已有可用命令。正式文档集中修改后一次 `npm run docs:check`。真实模型/付费服务/OS凭据/前台WebView由协调者预约与授权；本 P0-B 不运行这些命令。

## 12. 下一轮独立代码任务与集成依赖

共享契约以本文 §3/§6 为基础，由协调者指定唯一写入者；以下切片可以单独 review，但数据库、生产wiring和删除旧接口须成组集成，不能宣称各中间提交都能独立运行。

| 任务建议 | 唯一文件责任与产物 | 依赖 / 完成条件 |
| --- | --- | --- |
| P2-DB 迁移与历史 | `crates/dgos-db/src/migrations/` 新migration及mod.rs、`repositories/development_repository/{capabilities.rs,mod.rs,capability_tests.rs}`、必要session/run事务文件与 `repositories/user_data_repository.rs`；实现 §6.1 DDL/触发器和 §7.1 方法 | 可按本文独立派发；不改Provider/Agent/UI；DB事务fixture覆盖格式分离、revision0引用、正项目revision、原子open/run、归档、boot核实幂等及owner清理 |
| P2-PROVIDER 失效协调 | `crates/dgos-providers/src/{cli_launch.rs,manager.rs,lib.rs,cli_launch_tests.rs}` 及其凭据/模型变更实现文件；定义 §7.2 gate/change/sink/lease，不反向依赖agents | 可与DB独立派发；Provider自修订0有效；全部提交路径同步失效，opening注册竞争、失败不通知、旧事件不撤销新修订；宿主接线归P2-HOST |
| P2-CODEX 原生能力保留 | `codex_session.rs` Provider-only、防回退、删除混合逻辑及专属测试 | 依赖ProviderLaunch；以实施C观察的0.167.0新路径复验路由与原生能力，不复用0.166.0历史通过结论 |
| P2-CLAUDE 桥接切换 | `claude_session.rs`、`tools/agent-bridge`协议/schema、SDK私有覆盖与错误收敛 | 依赖同一ProviderLaunch；以2.1.319/SDK0.3.317新路径复验初始化/辅助模型/原生能力/无fallback |
| P2-HOST 管理与接口集成 | capabilities/Manager/executions/handoffs/takeover、可信OS boot adapter、Tauri lib/commands/导出、契约检查与直接相关规格；删除gateway | 消费§7.1/§7.2；唯一接线者；原子open、进程撤权、legacy只读、同引擎交接及boot恢复命令/UI闭环；账户agent_lease命名不变 |
| P2-UI Provider与历史 | development stores、列出的工作台组件及中英文、前端/preview fixture；删除CLI host confirmations | 消费已冻结DTO；无native默认、legacy只读、原对象草稿/unknown核实/关闭入口完整；P3/P4管理页消费native观察契约 |
| P2-VERIFY 集中验证 | 迁移/本地HTTP故障/真实CLI和桌面smoke资产，旧stage4专属路径清理 | 代码批次汇合后执行，不在并行编辑中反复构建；按§11分级报告；唯一实现状态由协调者回写 |

## 13. 发布与回滚

上线前完成自有旧运行停止或跨宿主 boot 恢复流程、可恢复数据库/原生历史备份、迁移故障回归、两家新路径真实CLI+本地端点的路由/防回退/清理证明及秘密泄露检查。供应商付费实测按另行授权记录覆盖，不作为每账户首配门槛。数据备份由正式升级流程执行，本设计不访问正式库/vault。

迁移仅向前。失败回滚本次事务并保持CLI执行域关闭，修复后重试；不能用“native模式”恢复可用性，也不能重新部署旧网关作为自动降级。数据库成功迁移后不得直接用旧应用读取并恢复混合会话；回退只允许支持只读legacy策略的修复版本。必须使用旧应用时须完整恢复升级前备份，并重新核实所有运行已停止，不能混合新库与旧进程。

原生 CLI 配置文件没有由P2改写，不存在P2配置文件补偿；P3/P4原生配置的预览、未知字段保留、外部修改冲突、原子写与中断恢复归其设计。监测仅使用现有安全诊断记录 migration状态、固定错误类别、会话/Provider引用和清理结果；不为本任务另建遥测系统。

## 14. 本轮交付核对与剩余风险

P0初稿/返工证据仅为当时静态阅读与核对。P2-PROVIDER 本轮实际HEAD为 `51c49fd7f1173209992f6160c52ba8035dc94068`；修改范围仅 dgos-providers 与本文，新增生命周期与边界测试源码。依 next-B 要求不在本TAB运行 tests/build/lint/formatter 或中途检查，批次验证由协调者集中执行；不把未运行测试或smoke记为通过。不修改DB/agents/application/workbench/native_extensions/src-tauri/前端/lockfile，不读写正式凭据或调用真实模型。

关键待实施风险：Provider通知须覆盖全部调用者并避免锁逆序；Claude私有接口/初始化路由及Codex可观测字段需在实施C观察版本复验；Manager须依据真实清理证据实现具体恢复入口，不能把普通旧历史缺boot升级为强制重启；旧消息仅有界历史。Provider切片不提升这些宿主事项的验证状态。

## 15. Codex 接入可靠性与交互完善

本轮用户授权在既有 D136 Provider-only 与 D127 原生会话边界内实施以下改进。不打包 Codex、不增加 ACP 或第二执行器、不自动升级本机 CLI、不回退原生账号。实现证据仅回写 V1 实现状态。

| 切片 | 实施契约 | 验证目标 |
| --- | --- | --- |
| 版本与启动诊断 | `--version` 仅识别稳定三段版本并展示实测值；已识别的本机版本不因不等于参考版本而被拦截。直接启动 `codex app-server --stdio`，完成 initialize、配置核对、thread start 与工作目录/Provider/model 身份校验后才发布会话；协议不兼容以握手失败返回，不发送提示词且不重试。参考版本仅用于文档与测试，不是运行白名单；可选原生操作逐项处理 Unsupported，不以版本号推断能力 | 0.166.0/0.167.0/较新稳定版通过相同握手fixture；无效版本、拒绝/断开/身份错配仍失败；真实 CLI 仅作无模型握手与协议证据，不自动升级、不读取原始秘密 |
| 会话同步 | 保留同一宿主状态与只读快照接口，增加可选 generation/after_seq 游标；有效游标只传新增事件及当前状态/审批，代次变化、历史缺口或非法游标返回全量重置。前端按序合并并淘汰宿主不再保留的事件，缺口重新读取，不触发任务发送 | 多会话隔离、迟到响应、账号重置、事件淘汰、审批独立更新；有界长历史载荷对比 |
| 能力来源 | 明确本机引擎及版本、DG-OS Provider/model、工作目录和原生 Skills/MCP；配置、清单发现、用户选择、实际执行分别表达 | 不将清单或选择表示为已执行；当前代次失效后不提供可操作权限 |
| 原生 Skill 输入 | 本节替代 §3.1 中本轮不新增选择器的范围限制。Codex 已建立会话可从真实发现结果显式选择最多8项 name/path；发送输入新增 skills，宿主按当前代次重验启用状态与唯一精确身份，转换为结构化 UserInput::Skill。不解析正文伪造选择，不自动每轮注入，不替 Claude 承诺该输入协议 | 恶意路径/禁用/过期拒绝；Skill身份纳入持久与内存幂等指纹；已派发回执先核实；草稿按会话隔离与未知结果不重发 |
| Claude 原生命令输入 | 自动准备的活动会话通过 supportedCommands 读取原名；仅完整清单内唯一且非 builtin 的命令可选，选择只将 `/原名` 加入当前消息草稿，参数由用户编辑并显式发送。仍走 text，不填 Codex 的 skills；D143/D144 允许有效精确工作区授权及模型下首发前准备，不发送占位回合 | 账户、项目、session/generation 或运行状态改变使旧清单失效；builtin/同名只读。来源为 unknown，不冒充确定 Skill 实例；正文/资源使用由原生事件另行验收 |
| MCP 会话管理 | 仅开放固定版本具有明确请求、取消与结果确认语义的操作。无可靠协议继续报告不支持并提供对应原生操作指引，不使用 shell、任意RPC或全局配置改写绕过 | 逐项核对官方协议；不可用原因、原生CLI操作指引及迟到/未知结果行为 |

同步优化优先减少全量历史复制与传输，不先建立推送订阅系统。现有600ms轮询的合成长历史载荷测量用于判断优化收益，不作为真实UI延迟或实机吞吐证据；运行态可缩短轮询间隔，非运行态仍保持状态可见，不改变隐藏页面保持运行的规则。

真实验证使用隔离项目、明确授权模型服务与可证明的原生凭据隔离。版本/help/schema、本地协议子进程、真实CLI连接本地故障端点、真实模型、桌面UI分别记录，不互相提升。若缺少隔离身份或真实模型授权，只能记录对应阻塞并完成其他可执行验证，不以HOME重定向声称默认钥匙串隔离。

### 15.1 批次 A：可靠运行闭环

按用户批准的四批次推进方案，批次A仅加固既有执行路径，不新增steering、Plan模式、Review、OAuth或第二执行器。真实CLI与外部模型的完整验收延期至A～D完成后集中执行；本批仍执行必要的确定性故障回归与隔离协议/界面smoke，不访问正式凭据。

- 传输按响应、服务端请求和通知分类；已超时/取消请求的迟到响应不得进入审批事件流。pending注册有界且随请求取消释放；普通事件队列饱和时明确使连接失效，不无界缓存、不静默丢弃审批或终态。IO失效独立于普通事件队列和child reap通知，会话立即失效；进程回收仍以真实wait结果为准。
- 持久化run准入完成后的发送由宿主监督，不随界面/调用future消失而遗失确认处理；不自动重发原生变更。运行终态独立于可裁剪的展示事件保留，按run身份交给持久化监督，前一run未持久化完成不准入下一run。
- checkpoint/终态落库失败不终止原生状态观测；保留最后已观测结果，以有界退避重试幂等的本地投影写入，不重试模型请求。快照新增可选`persistence_error:'storage_failed'`，与原生status/error分离；未收敛时禁止新发送，但允许原生中断与关闭。退出时未完成监督不得被丢弃后报告成功。
- 首次发送保留任务、目录信任、Provider修订、账户与generation约束。确定发生于run准入前的输入/控制拒绝允许用户修正后显式发送；未知派发结果继续只读核实，不自动重放。中断独立于一般action等待，绑定当前runtime/run，确认请求后等待原生终态，不将RPC应答伪装成已停止。
- 定向验收覆盖迟到回包、队列饱和、IO结束早于reap、调用者取消、终态事件被裁剪、落库失败后同进程收敛，以及首发与中断的代次隔离。局部fixture证据不提升为真实CLI、外部模型或完整发布验收。

### 15.2 批次 B：原生执行信息可读化

2026-11-09：原四批次方案中段无法从归档恢复，用户明确授权重建 B～D 范围并连续实施。以下是替代范围，不冒充丢失的原清单。沿用 D136/D127 边界，不新增第二执行器、任意 RPC、OAuth、原生账号回退或自动升级；本轮不新增原生 Review 操作。

- 只投影原生公开 reasoning summary，按 run/item/summaryIndex 合并增量；对应完整 summary 替换同项流，不展示原始 reasoning content，不编造缺失摘要。
- Plan/diff 按 run 保留更新版本，明确最新与历史；保留步骤状态和说明，计划完成、回复完成与 diff 展示都不等于补丁已应用或产品验收通过。畸形字段保留检查入口。
- 上下文面板区分最新回合与累计 token，展示输入、输出、缓存、推理分项。只使用原生窗口，累计量不算占用百分比；执行中保留最近原生统计的时点。D144 扩展费用、耗时及压缩，实际费用、可靠原生价格预估及未知分别表达，详见 §17；不从模型名称估价。
- 验收覆盖跨 run/item 隔离、流与快照替换、多次计划/diff更新、异常统计和缺窗口。

### 15.3 批次 C：有身份约束的原生回合交互

- 运行中追加指令使用原生 `turn/steer`，宿主检查 owner、Provider租约、目录、generation 与 run；捕获原生 `expectedTurnId`，不能把旧请求转发下一轮。请求身份绑定原文本，一次派发；调用者取消后仍监督应答，未知结果仅核实、不自动重发。界面保留未确认草稿，不把接收确认当执行完成。
- Plan/default 通过 `collaborationMode/list` 的实际发现开放，采用原生内置指令和当前绑定模型，前端不能传任意 developer instructions 或模型。使用实验协议必须显式初始化 opt-in；不支持时仅禁用对应控制，普通发送保持。D144 的 plan/deep 另施加原生受限执行策略，不能只靠 Plan 名称或提示词保证不写入，见 §17。
- 模式选择是下一回合意图，实际状态由原生确认更新；拒绝/未知保留意图，不能显示成已生效。会话/账户切换不得让迟到应答清空新草稿。
- 首条消息前由有效 Provider/模型和精确工作区 agent 授权自动准备，替代独立“连接并配置”入口；不发送占位消息或模型回合。输入保持可编辑，真实能力就绪后直接选择模式和推理；准备不消费草稿或文件引用，切换成功后迁移最新输入。双击不重复创建，未知结果只核实原会话，不自动重启或发送；缺失能力就地显示简短不可用原因。
- 验收覆盖旧代次/旧回合、同ID不同输入、未知不重发、能力缺失、绑定模型不变、Plan/default切回以及导航隔离。

### 15.4 批次 D：长时间运行资源收口

- 自有进程确认退出、清理事实持久化且本代run投影已收敛后，释放重量级backend及原生回执；保留有界轻量终态展示，持久历史和显式恢复入口保持。
- 正常运行期间消费已完成监督句柄；失败、panic、超时仍影响关闭结论，不能通过删除句柄隐瞒。退出未确认或落库未收敛的对象保留必要租约/占用，不依据保存PID杀进程。
- 清理绑定slot/generation，旧清理不能移除新代次；跨账户不可读取终态缓存。验收覆盖反复开关、自然退出、DB故障、活动run、迟到清理与恢复。

B～D 编辑阶段不逐文件执行检查；公共契约确有必要时做局部核对，全部切片汇合后集中定向回归、构建与隔离行为验证。真实CLI/本地故障端点与外部付费模型分级记录，未获具体模型服务授权不调用正式凭据。

## 16. Codex 桌面能力补齐（三批次）

2026-11-10：用户授权按核心正确性、高频原生交互、长任务与扩展管理依次实施，并在批次汇合后集中检查、真实桌面验收与review修复。本节扩展§15此前未纳入Review等操作的范围，不改变D136 Provider-only、原生执行、宿主所有权及未知结果不重放边界。原生协议以本机导出的0.167.0 schema及实际运行行为核对，实验方法明确标识；方法缺失只禁用对应能力，不伪造完成。

| 切片 | 实施边界 | 验收目标 |
| --- | --- | --- |
| P1 提问与审批 | 修正为原生`item/tool/requestUserInput`；问题、选项及审批决策按原生请求呈现，回答绑定owner/session/generation/request，不新增原生未请求的权限 | 标准原生请求能够回答；过期、重复、跨代次及只读扩权拒绝 |
| P1 开发闭环 | 复用已授权Provider及隔离项目，真实多文件修改、实际审批、失败测试修复、宿主独立复验及关闭恢复；不操作用户项目 | 当前代码真实行为，既有历史证据不能替代 |
| P2 图片 | 显式选择/粘贴、预览、移除后随消息发送；宿主校验类型、大小、归属并固定内容；图片参与请求身份，不接受远端URL或任意本地路径读取 | 正常图片真实输入；超限、伪类型、旧会话、重复请求及失败草稿保留 |
| P2 Review/压缩 | 原生`review/start`与`thread/compact/start`；Review纳入运行台账和中断，压缩明确等待原生结果；不将请求受理当执行完成 | 实际审查结果与压缩状态；无隐式普通消息替代 |
| P2 分叉/回退 | 原生`thread/fork`与`thread/revert`；只接受宿主已归属线程/turn；分叉保持Provider/目录绑定且不自动续发；回退显式确认，只改变会话历史、不撤销磁盘，历史run审计保留 | 原线程保留、目标归属与恢复正确；旧审批失效、未知操作不重复 |
| P3 子智能体/后台命令 | 子线程必须来自已归属父线程的原生关系，不信任前端任意threadId；专用树、状态与受支持控制；后台命令使用原生processId与所属thread，不把宿主PTY冒充原生进程 | 父子归属、切换查看、实际命令枚举及逐项停止；跨线程拒绝 |
| P3 队列 | 原生queue增删改排及显式启动，身份、草稿、未知回执与取消独立；原生自动启动事件同样必须进入宿主监督，不绕过Provider租约与run台账 | 顺序、取消、重复/未知与重启边界；不把steer伪装为排队 |
| P3 Skills/MCP/认证 | 关联配置提交、当前代次发现、生效与实际调用证据；仅开放能界定作用域和完成/取消的原生控制。复用已有认证生命周期，URL/回调及凭据不进入普通事件/日志；停止交互不承诺撤销已写入凭据 | 真实原生能力发现/调用；取消、过期、迟到结果和撤权；无协议保证者明确限制 |

共享约束：所有新增变更操作使用类型化参数白名单、请求身份及有界回执；不得向renderer开放任意RPC或命令。前端采用现有设计token、组件与账户/项目/session草稿隔离；原生工作管理作为渐进展开区域，不挤占主要代码与对话。图片、子线程和认证的私密内容不进入普通诊断。开发期间只对影响后续实现的高风险契约做必要探针，汇合后集中测试、构建、真实界面与真实引擎验收，修复后仅复验相关失败及其集成边界。

## 17. D144 输入、模式与原生用量增量

依据已冻结 [D144](../../06-决策记录/V7-冻结决策.md#d144-cli-agent-直接配置精确工作区自动信任与用量交互)。本节取代旧逐次信任、Plan 不约束写入及不启用全自动的规则；Provider-only、账户/目录归属、撤销和未知不重放边界保持。活动精确 workspace 的自动授权、持久拒绝与重新启用以[接口契约 §17.11](V1-接口契约.md#1711-d143-cli-agent-工作现场目录授权与退出)为准，terminal/service 不随 Agent 自动授权。

五项菜单分别组合 `permission_mode` 与 `collaboration_mode`，不是一个新增的原生 mode 枚举；`read_only` 仍是宿主权限值，不额外占一个菜单项。字段、空值和能力返回以[接口契约 §17.9](V1-接口契约.md#179-codex-接入完善增量读取与原生-skill-输入)为准。

| 选择 | 原生应用与边界 |
| --- | --- |
| 手动确认 | `default` 执行权限及默认任务方式；审批按原生可信规则，不承诺每次读取弹窗 |
| 自动编辑 | `accept_edits`；Claude 映射 `acceptEdits`。Codex 当前适配不支持此策略，显示简短不可用原因，不以全自动代替 |
| 规划 / 深度 | `plan/deep` 任务方式，保存的执行权限意图可仍为 `default`。Codex 使用原生 plan、readOnly sandbox、networkAccess=false 及 never 审批；宿主拒绝写入/扩权批准。Claude 使用原生 plan，深度追加 planModeInstructions；宿主拒绝其他副作用批准，仅允许原生问答及显式 ExitPlanMode。Claude 不据此宣称 OS 沙箱，其原生计划文件/元数据仍遵循 CLI 规则 |
| 全自动 | `full_auto`，菜单点击明确授权当前会话。Codex 为 never 审批并保留原 workspaceWrite 根与网络限制，不使用 dangerFullAccess；Claude 为 bypassPermissions，只有显式选择后才开启对应启动许可。不得成为其他项目或新会话默认，不消耗既有 pending |

深度先澄清目标、比较方案，再产出计划，推理强度保持独立。离开规划恢复所选执行权限；不能因保存的 permission 仍为 default 就取消规划期间的原生限制。Claude SDK 0.3.317 / CLI 2.1.319 为本轮核对写协议组合，不据此推断其他版本或全部模型支持；模式能力以当前代次实际返回开放。拒绝/未知保留意图和草稿，不把选择、schema 或原生受理回执当实际工具执行证据。

推理使用原生真实离散档位及显式默认 null，不以默认冒充“关闭”，不按模型名称补档。响应语言、精简程度和下一步建议经 `response_preferences` 作用于后续回复，不改写用户 text；跟随修改、流式展示和代码高亮只改变呈现，主题复用应用外观。关闭流式展示仍实时处理审批、取消、工具和失败。偏好在既有账户私有工作现场保存，不另建状态源；保存失败就地反馈并恢复已确认值。

用量仅消费原生回执：Codex 最新 last 与累计 total 分开，缓存/推理为子集，不重复相加；Claude 上下文取最近主响应用量及同回合、同模型窗口，累计取最新 model_usage 快照，不能把多份累计快照相加。实际费用与原生可靠价格预估明确区分；缺少价格依据、窗口、用量或完整耗时均显示未知，历史裁剪不能冒充完整会话统计。

压缩复用 `observe_agent_compression` 当前代次能力读取、`control_codex_operation` 的类型化 compact 动作及原请求核实，沿账户、Provider、目录、generation 和空闲检查。Codex 使用 `thread/compact/start`，首次能力依据精确已核对 0.167.0 schema 或当前进程既有直接协议成功证据，不按版本大小推断，请求受理不等于完成；Claude 仅在原生 supportedCommands 确认 builtin `/compact` 后调用，并关联原生结果。版本/schema、能力发现、协议 fixture、真实 CLI 执行和桌面交互分别留证；未支持时局部禁用，不以普通提示词、前端调低进度条或旧压缩成功记录替代当前完成。失败/未知不修改占用、不自动重发，刷新占用必须等待新的原生用量。

## 变更记录

| 日期 | 变更 | 关联决策 |
| --- | --- | --- |
| 2026-11-07 | P0-B：Provider-only运行契约、旧混合历史迁移、两家防回退与撤权、删除调用清单和P2代码切片 | D136、D130、D131 |
| 2026-11-07 | P0-B审查返工1：允许Provider自身修订0；移除逐组合付费验证门槛；分离数据/本地清理/外部unknown并补boot恢复出口；复用capability表、合并binding清理、补最小DDL和可独立派发接口；更新当前候选版本，沿用agent_lease | D136、D130、D131 |
| 2026-11-07 | P2-PROVIDER：收窄历史重启门槛；实现共享提交/spawn gate、opening租约、同步失效/锁外通知、变更路径接线及边界测试源码；§7.2记录实际API、宿主接入与集中smoke方案 | D136、D130、D131 |
| 2026-11-08 | 第四阶段：Stage4 原生资源事务、独立读取/审批/执行/结果回送证据及新版 artifact 恢复边界；驱动能力限制和实际证据见 V1 实现状态 | D136、D131 |
| 2026-11-10 | D144：收口活动精确工作区自动信任、五项模式的原生限制、响应偏好与用量/压缩证据边界；取代重复确认和旧 Plan 规则，不回写实现状态 | D144 |
