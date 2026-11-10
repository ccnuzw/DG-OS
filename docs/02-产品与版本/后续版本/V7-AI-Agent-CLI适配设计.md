# V7 AI Agent：Codex 与 Claude 独立会话适配

> 状态：技术任务设计。用户已按 D127 授权本切片；不代表 V7 整体门禁通过。旧单次只读切片见[历史记录](../../99-历史归档/2026-11-03-Agent单次CLI切片.md)。

## 1. 范围与依据

采用 [D127](../../06-决策记录/V7-冻结决策.md) 与 [ADR-0004](../../06-决策记录/ADR/0004-Codex与Claude独立会话适配.md)，运行绑定以[D136](../../04-技术架构/当前版本/D136-CLI能力切换技术设计.md)为准，输入、信任、模式及用量增量按[D144](../../06-决策记录/V7-冻结决策.md#d144-cli-agent-直接配置精确工作区自动信任与用量交互)。当前只支持 Codex 和 Claude，Gemini 延后。使用本机 CLI，不随应用打包 CLI。普通 Provider 聊天不因此获得 Agent 能力。

- Codex：Rust 直接使用 `codex app-server --stdio`，独立处理 thread、turn、item、审批与恢复。
- Claude：Rust 管理打包的 `dgos-claude-bridge`，官方 TypeScript Agent SDK 0.3.317 通过 `pathToClaudeCodeExecutable` 调用本机 claude。桥接程序内包含 SDK 和 JS 运行时，不包含 Claude CLI。
- 两套原生状态、配置、审批响应分别维护；共用用户归属、工作区信任、进程监督和基础显示组件。
- 接口：Codex [App Server](https://developers.openai.com/codex/app-server/)、Claude [SDK](https://code.claude.com/docs/en/agent-sdk/typescript)、[程序化调用](https://code.claude.com/docs/en/headless)。按实际安装版本验证兼容性，不承诺所有官方客户端功能。

## 2. 运行与配置

用户进入已打开项目的活动精确工作区后，由宿主核对账户、项目、task/workspace 与真实目录身份并幂等建立 Agent 信任；不再显示授权按钮或二次确认。手动撤销持久拒绝，显式重新启用直接授予；目录替换须重新定位，terminal/service 独立授权。选择有效 Provider/模型后自动准备，CLI 路径复用应用设置；配置加载可能在第一条消息前执行 Hooks 或启动 MCP。Codex 默认 workspace-write/on-request；Claude 使用 Claude Code 系统提示预设及 user/project/local 设置。D144 全自动仅在用户显式选择本会话后启用对应原生策略，不继承为新会话默认、不批准旧 pending；规划/深度另施加原生受限策略，映射与 Claude 非 OS 沙箱边界见[D136 §17](../../04-技术架构/当前版本/D136-CLI能力切换技术设计.md#17-d144-输入模式与原生用量增量)。

CLI 优先使用显式绝对文件路径，其次 PATH、用户 `.local/bin`/`.npm-global/bin`/`.bun/bin`、`/opt/homebrew/bin`、`/usr/local/bin`。Unix 候选必须为有执行位的普通文件（允许指向有效文件的符号链接）；自动发现跳过无执行位文件，显式指定则返回 invalid_executable。实际权限、架构或解释器导致的启动失败仍由进程启动报告。不经过 shell，不接受前端 argv、环境或原始 RPC。认证由原 CLI/SDK 处理，凭据不进入前端和索引。

当前新会话仅使用 DG-OS 精确 Provider/model/revision，Skills/MCP 使用 CLI 原生体系，不提供 native Provider、混合受管模式或原生账号回退。凭据仅交给受信 CLI 进程环境及桥接私有 stdio，原生 Shell/Hooks 仍使用本地用户权限；Provider 绑定快照、能力发现和实际执行分别取证。D135 阶段4及其 capability_revision=0/正数规则只作历史迁移依据，不能用于新会话准入；当前绑定与旧历史只读策略见 D136，接口及字段以[接口契约 §17.9、§17.11](../../04-技术架构/当前版本/V1-接口契约.md#179-codex-接入完善增量读取与原生-skill-输入)为准。

开发和 Tauri 构建前执行 `npm run agents:build`，要求构建机有 Bun。依赖安装使用 `--omit=optional --ignore-scripts`，不拉取 SDK 自带 Claude 可执行文件。桥接产物放入资源目录 `agent-bridge/`；macOS 已有本地运行证据，其他平台的分发和进程树行为不能从该证据推断。

Claude 发布认证约束：按 [Agent SDK 官方概览](https://code.claude.com/docs/en/agent-sdk/overview.md)，未经事先批准，第三方产品不得提供 claude.ai 登录或使用其额度，应使用官方文档所列 API key 认证方式。当前本机技术验收不能证明第三方分发授权；发布前必须明确合规认证路径或取得批准。桥接不打包 Claude CLI，不据此推断其认证使用不受限制。

## 3. 宿主契约

全部入口仅允许可信 main 窗口、Workbench/TasksRun，用户由宿主派生。

| 命令 | 输入 | 输出 |
| --- | --- | --- |
| `list_agent_sessions` | 无 | 当前用户会话索引 |
| `open_agent_session` | `input` 为 tagged `create` / `resume`；精确字段见下 | 当前代次会话快照及 Provider 绑定 |
| `get_agent_session` | `sessionId,cursor?` | 有界完整/增量快照，含 generation、controls 与 active_run |
| `send_agent_message` | `input:{session_id,generation,request_id,text,settings?,references?,skills?,images?}` | 已提交确认；settings 真实字段见接口契约 §17.9 |
| `respond_agent_request` | `input:{session_id,generation,request_id,response}` | 原生请求响应确认 |
| `interrupt_agent_session` | `sessionId,generation,runId` | 目标本轮中断请求确认，实际终态由原生事件决定 |
| `close_agent_session` | `sessionId` | 限时进程回收结果 |

`engine` 为 `codex` 或 `claude`。create 为 `{intent:'create',id,engine,options,task_id,project_revision,provider}`，options 含 workspace、resume_id、executable、model、trust_workspace，Provider 为精确引用；resume 为 `{intent:'resume',session_id,trust_workspace}`。新入口传 trust_workspace=false，由宿主消费当前精确工作区有效 agent 授权。CLI 路径由宿主应用偏好解析；renderer 不得用任意 resume_id 恢复本机其他会话，恢复只接受宿主索引 DG-OS id，再由宿主找到 native ID。新建 ID 与消息 request_id 必须 UUID；相同会话不能改成另一引擎。

会话快照包含 native_id、workspace、status、events、pending、error。状态为 starting/idle/running/waiting/closed/error/unknown；idle 代表可以发送下一轮，不表示进程结束。原生失败终态为 error；失联且无可靠终态为 unknown。发送未知结果不得自动重试。重复消息 ID 使用输入摘要避免同一进程内重复提交；不承诺跨重启恰好一次。

Provider 项目配置修订为正数，Provider 自身 config_revision 允许 0，两者不得混淆。恢复沿原绑定重验 Provider、epoch、账户、目录及 CLI 身份，不接受替换配置；旧混合或历史原生会话只读，需审查摘要后新建交接。配置准备、清单发现、输入选择、正文/资源读取和工具执行分别表达，不能从 prepared/connected 推断已执行。

消息最多 64 KiB UTF-8。事件单行最多 4 MiB；显示快照保留最近 2000 条、约 16 MiB 内容。历史原文仍由 CLI 保存，DGOS 只保留有界展示投影。审批 pending 独立保存，不能因显示事件淘汰而丢失。

D144 的推理、响应偏好、用量及压缩沿[接口契约 §17.9](../../04-技术架构/当前版本/V1-接口契约.md#179-codex-接入完善增量读取与原生-skill-输入)。推理仅取真实离散档位，null 表示原生默认而非关闭；语言/精简/建议作为独立响应设置，不改用户 text，跟随修改/流式展示/代码高亮只作用呈现，主题复用应用外观。用量来自原生回执，未知不填零、累计不当上下文、费用实际与可靠价格预估区分。压缩复用现有类型化 compact 动作，仅在当前账户、Provider、目录、generation、空闲及能力校验后派发；原生受理不等于完成，失败/未知不改占用、不自动重发。

## 4. 生命周期、归属与持久化

阶段1按[D134及项目任务契约](V7-项目任务基础技术契约.md)将元数据迁入主SQLite；`agent-sessions.json`只作为一次性导入源并保留原字节，不再写入。无任务历史进入待归类，不创建虚构任务。新会话绑定自有项目任务；恢复保留原CLI路径、模型与原生ID，不替代CLI原生历史。迁移或索引损坏关闭本域准入，保留旧数据；不导致整个应用崩溃。原JSON原子写入设计为迁移前历史实现。

页面通过快照刷新订阅运行状态，页面卸载不取消任务。账户会话撤销立即取消 lease；所有后续操作重新校验归属和 lease，旧账户不能向新会话投递。应用退出关闭准入并清理。Unix 子进程使用独立进程组，TERM 后限时 KILL；主动脱离进程组的第三方进程不在此保证内，取消也不撤销已发生的文件或外部副作用。

中断使用 Codex turn/interrupt 或 Claude SDK interrupt；关闭才结束进程。Claude 透传原生 terminal_reason，aborted_tools/aborted_streaming 表示当前轮被中断，恢复 idle 以允许下一轮，原始 is_error 和终态仍保留在结果事件中；其他失败继续为 error，不用本地“已点击中断”覆盖原生错误。原生历史保存失败或会话尚未持久化时，恢复可以失败，不伪造空历史成功。

## 5. 两套审批与展示

Codex 保留命令/文件审批、工具问题和 MCP elicitation 的原生请求 ID；响应须匹配 pending 类型。命令审批校验 availableDecisions，已结束请求不能重答。Claude 使用 canUseTool 回调，返回 allow/deny，AskUserQuestion 回答仅合入该请求原输入，前端不能修改其他工具参数。

界面显示原生工具与文件信息，文本按文本渲染，不执行引擎输出的 HTML。普通消息、流式文本、命令输出、文件差异与详情共用组件；两家的审批结构与响应分别处理。模型和配置属于对应引擎；DGOS Skills/MCP 不自动同步为原引擎配置。

## 6. 分支与验证

| 分支 | 行为 | 验证 |
| --- | --- | --- |
| B01 | 宿主核实活动精确目录、UUID 与归属后自动建立未被拒绝的 Agent 信任；撤销不重授、目录替换拒绝 | 宿主自动激活/持久撤销/身份冲突测试及真实桌面；不沿用旧每会话勾选门禁 |
| B02 | 原生握手后登记索引，失败清理进程 | 真实本机 CLI 握手 smoke |
| B03 | 同一会话继续发送，原生事件更新显示 | Codex 真实两轮，Claude 真实多轮与恢复 |
| B04 | 审批匹配 pending，拒绝无效与迟到响应 | Codex 协议 fixture、Claude 真实 Write 拒绝/迟到批准拒绝、UI 原生拒绝测试 |
| B05 | 原生中断后保留会话，关闭后允许恢复 | Codex fixture/真实恢复，Claude 真实审批期间中断并继续输入；流式中断类别由单元回归覆盖 |
| B06 | 账户撤销拒绝访问、退出回收进程 | Rust lease/归属测试、真实关闭 smoke |
| B07 | 无确认或原生失败不假装成功、不自动重发 | UI 失败不重试测试、Claude 实际 model_not_found 诊断 |

## 7. 验证记录

以下保留 D127 既有切片证据，旧单次 runner 的测试结果不作为新路径证据；其中旧信任门禁、默认权限及 UI 结果不证明 D144 的自动授权、全自动、规划/深度、偏好或压缩已验证。新增实现与分级证据只维护在[V1 实现状态](../当前版本/V1-实现状态.md)，本次契约更新不提升其状态。

- 本机 Codex 0.166.0、Claude Code 2.1.317：真实握手、获取 native ID、关闭、跨用户查询拒绝通过；无模型调用的启动 smoke 已运行。
- Codex：临时工作区真实两轮上下文对话通过，关闭后恢复获得原历史；真实创建 amber.txt 并用命令读取，观察文件内容与工具事件通过。
- Claude 首轮历史阻塞：旧配置模型 `kiro-2/claude-opus-5.5` 不存在或无权限；该路径已验证为 error。用户于 2026-11-04 更正为 `kiro-2/claude-opus-5-5[1m]` 后，模型可正常返回结果，重测见下。
- 官方 Claude SDK + 无网络 CLI 协议 fixture：权限请求/拒绝、第二轮输入、interrupt、close 退出码 0；不升格为真实 Claude 工具执行证据。
- Rust 原生会话回归覆盖归属、重复/冲突输入、原生审批拒绝、迟到响应、恢复索引、interrupt 和 lease 撤销。
- UI 定向测试覆盖信任门禁、页面卸载保留运行、发送失败保留输入且不自动重试、Claude 原生拒绝响应。浏览器实际组件 smoke 已观察会话和审批表面，使用 IPC fixture，不代表 Tauri IPC E2E。
- 宿主编译、125 项 IPC 静态契约、前端构建与桥接 TypeScript 类型检查通过。UI 定向 4/4 通过；Rust 会话回归 1/1 通过（包含多个生命周期与权限场景）。
- `cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture`：真实 macOS WebView 8/8 通过，覆盖页面加载、两个真实 CLI 的握手/快照/关闭以及所属用户目录。首次 smoke 误用了 CommandResponse.data 读取器，实际命令返回值正常；修正测试读取后通过。该 smoke 不调用模型、不覆盖完整界面端到端审批。
- 文档检查：103 份文档、126 条决策，0 错误/警告/提示。临时模型/协议/UI smoke 入口已移除。
- 未覆盖：签名发布、跨平台清理、完整 MCP/Skills/插件组合、真实流式生成期间中断、完整界面端到端审批；不能以本轮验证声称全部上游功能已经接入或通过。

### Claude 模型配置修正后重测（2026-11-04）

- 使用当前 Rust AgentManager → 官方 SDK 辅助程序 → 本机 claude 的实际路径；options.model 留空，采用用户已修改的本机配置。重新构建辅助程序，运行临时 `claude_retest` 入口；未修改生产代码或全局模型配置。
- 真实第二轮正确返回先前提供的 `amber-7429`；关闭后用新的 AgentManager 从文件索引重新加载，恢复 native ID 一致，载入四条历史消息，恢复后的模型仍正确返回口令。
- 真实 Write 请求通过宿主权限回调，测试仅批准临时目录 proof.txt 的指定内容；文件成功创建。真实 Bash 调用 `cat proof.txt` 返回 `amber-7429`，宿主直接读取文件亦匹配。Bash 由原生策略放行，不能声称本轮 Bash 也触发了审批回调。
- 未通过项：两个新会话第一轮均返回通用问候，没有遵守“只输出口令”。检查其中一次原生会话记录，用户提示完整送达，记录的模型为 `kiro-2/claude-opus-5-5`。后续上下文能读取该口令，但首轮偏离原因尚未定位，不能据此直接归因为 DGOS、SDK 或上游服务。
- smoke 整体因首轮断言失败退出 1，未放宽断言为全通过；多轮上下文、持久化恢复、文件写入、命令读取和清理分别有成功证据。临时工作区与 smoke 入口已移除，原生 CLI 留存自己的测试会话历史。本轮未重跑无关前端/Rust 套件或 WebView smoke。

### 原生 CLI 对照与控制路径修复（2026-11-04）

- 独立 CLI 对照：在独立临时目录运行 `claude -p 'Remember this exact token: amber-7429. Reply only with that token. Do not use tools.' --output-format json --no-session-persistence`，普通模式和追加 `--safe-mode` 均回复通用问候。现象不依赖 DGOS 或 SDK 桥接；仍需区分本机 CLI 与当前 Kiro 服务的原因。两种模式追加 `--tools ''` 均返回 Kiro 400 请求格式错误，因此无工具模式不能作为本机有效基线。未添加隐式重发或首轮问候规避逻辑。
- 真实 Write 拒绝后目标文件不存在，迟到 allow 返回 request_expired。随后在等待 Write 审批时中断，修复前宿主为 error/engine_failed，pending 已清空。
- 诊断确认原生结果为 is_error=true、subtype=error_during_execution、terminal_reason=aborted_tools。桥接原先丢弃 terminal_reason；现透传并按原生中断类别允许继续，保留原始结果。一次诊断的 errors 同时包含辅助模型 claude-haiku-5.5 不可用，不能将该附带错误当作中断的结构化终态，也未修改用户全局配置。
- `npm run build --prefix tools/agent-bridge && cargo run --offline -p dgos-agents --example claude_controls && cargo test --offline -p dgos-agents`：修复后真实拒绝无文件、迟到批准拒绝、中断清空 pending 且无文件、同一会话继续回复 CONTROL-OK、进程清理全部通过；Rust 2/2 通过。回归覆盖 aborted_tools/aborted_streaming 可继续，以及 API/模型/未知错误不能误判为可继续。
- 桥接 TypeScript 定向类型检查通过。临时诊断桥接、控制路径 smoke 入口和工作目录已清理。该证据属于真实 Rust/SDK/CLI 路径，不代表界面端到端或签名发布验收。

### 实际组件与原生扩展验收（2026-11-04）

- 临时入口挂载生产 AgentWorkbench 与生产 store，在打包来源的 macOS WKWebView 中通过 DOM 输入/点击操作，注册真实 Tauri Agent 命令，使用临时宿主索引和真实本机 CLI；未替换 IPC 返回值。该入口不含完整工作区外壳，不验证标签关闭保护、账户切换、原生鼠标命中或 VoiceOver。
- `npx vite build --config vite.agent-acceptance.config.ts` 后运行 `DGOS_AGENT_UI_ACCEPTANCE=1 cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture`：4/4 聚合检查通过，覆盖加载、表单及两家完整组件场景。两家均通过创建、发送、回答显示、中断后继续、卸载/重新挂载保留历史、关闭及原生会话恢复；Claude 额外通过实际 Write 的界面批准、拒绝和等待审批时中断，宿主核对批准文件内容及拒绝/中断文件不存在。Codex 中断的是运行中 sleep 命令。
- Codex MCP 增补：以显式可执行路径传入临时启动器，只向本机 codex 注入单个本地只读 stdio MCP 配置，不改全局配置；追加 `DGOS_ACCEPTANCE_CODEX=<临时启动器绝对路径>` 运行同一目标，3/3 聚合检查通过。界面 Deny 拒绝原生 MCP 请求，后续 Allow once 允许调用并显示 MCP-SAPPHIRE-7186；随后继续输入、组件重挂和恢复均通过。首次提示禁止了必要的工具发现，模型未调用；允许工具搜索后验证通过，不属于生产缺陷。
- Claude 原生扩展：直接运行打包 dgos-claude-bridge，通过同一 open/send/respond 协议驱动官方 SDK 与本机 CLI。隔离目录中的 `.claude/skills/dgos-proof/SKILL.md` 被 `/dgos-proof` 加载，返回只在技能中定义的 SKILL-QUARTZ-6384；`.mcp.json` 与项目 enabledMcpjsonServers 加载本地 MCP，真实 ToolSearch 找到工具，权限回调批准后返回 MCP-SAPPHIRE-7186。桥接关闭退出 0。此扩展场景未经过 Rust/Tauri/UI，不能升格为扩展的完整桌面验收。
- Codex 原生 Skill：独立 App Server 在隔离目录加载 `.agents/skills/dgos-proof/SKILL.md`，`$dgos-proof` 返回 SKILL-QUARTZ-6384。直接 App Server MCP 初次对照拒绝了审批，无 token；有效批准路径由上述真实 UI 增补覆盖。
- 视觉证据：浏览器打开实际组件并截图确认初始表单可见；该浏览器没有 Tauri IPC，不作为运行证据。WKWebView 行为以 DOM 结果及真实文件/模型输出为证据，桌面截屏未取得有效窗口画面。早期外部 dev URL 的 smoke_report 超时，切换打包来源后通过，未放宽生产权限。
- 未修改生产行为。临时 HTML、Vite 配置、Rust 验收入口、启动器、MCP/Skill 工作区及对应生成文件清理；保留原有握手 smoke。真实模型会话历史由 CLI 自行保留。未运行无关全量测试。

### 完整工作区生命周期与 MCP 取消（2026-11-04）

- 使用本轮 `npm run build` 的生产 dist，完整 App/AccountGate/工作区标签/ModalHost，在 macOS WKWebView 中运行；临时迁移数据库、真实 AccountService/HostSessionPort/SessionResolver 和 AgentManager，真实本机 CLI。注册本次必需的账户、外观及 Agent 命令；模块目录等无关宿主服务未注册，因此不代表所有工作区模块集成通过。
- 临时入口命令：`DGOS_AGENT_WORKSPACE=1 cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture`。首次运行的 Codex 场景通过：运行中切换桌面不停止会话，关闭确认取消保留标签，停止并关闭等待宿主 closed，重新打开保存会话可继续对话，未发送草稿在取消关闭后保留。
- Claude 最终复验 7/7 聚合检查通过，覆盖上述同等工作区行为，以及真实 UI 注销、登录页重载、Agent 工作面消失、注销后旧会话 IPC 拒绝、宿主移除所属活动会话。注销前用真实 IPC 恢复 idle 会话，证明清理活动运行实例；不扩大为双账户切换或全部退出竞争时序证据。
- 失败过程如实保留：第一次“等待许可”使模型只口头等待；第二次模型延续首轮 no-tools 约束；第三次将 blocked.txt/NO 误判为敏感任务。改为首轮普通问候、第二轮普通 notes.txt 便笺任务后进入真实权限请求并通过。另修正临时脚本按钮名“退出登录”为实际“注销”。未修改生产提示词、权限策略或自动重试行为。
- 原生 MCP 执行中取消：隔离只读本地 stdio 服务器先写 started 标记，等待 35 秒再返回，主测进程确认 started 后才调用原生 interrupt。Claude 使用打包桥接的 open/send/respond/interrupt 协议，Codex 使用独立 App Server 原生 RPC；两者服务器均收到 notifications/cancelled。Claude 终态 aborted_tools，随后返回 AFTER-CANCEL-OK；Codex 终态 interrupted，随后返回 CODEX-AFTER-CANCEL。两个客户端退出 0。
- MCP 证据是原生协议/官方桥接级别，未经过本轮 UI，也不证明外部副作用回滚或所有服务器主动停止。两家均未修改全局设置，临时服务器与项目配置已清理；临时工作区验收入口移除并恢复原有 smoke。生产代码无本批改动；前端完整构建通过。

### 双账户与连接竞争边界（2026-11-04）

- 临时验收入口命令：`DGOS_AGENT_BOUNDARIES=1 cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture`。使用上一批已构建的生产 dist、完整 App/AccountGate、macOS WKWebView、真实账户服务和 Agent 命令，临时数据库创建 alice/bob 两个账户；无前端或生产代码变更，不重复构建。最终退出 0，2/2 聚合检查通过，内部两引擎共 8 组边界场景通过。
- 两引擎分别经真实 CLI 握手建立 idle 会话。通过 UI 切换用户时，错误密码显示错误且原账户、原会话仍可访问；正确密码切至另一账户后，旧会话不出现在列表，读取、发送、中断、关闭、恢复五个真实 IPC 均被拒绝，旧 owner 的运行快照也不可访问。切回原账户，只列出自己的保存记录。此处不证明旧进程 PID 消失，也未覆盖审批响应越权或运行中切换。
- 新账户分别连接真实 Codex/Claude，两个 close_agent_session IPC 并发调用均返回 true，随后快照为 closed；此证据限定 idle 会话的重复关闭，不扩大为发送、审批、恢复与关闭的所有竞争。
- 连接中注销：UI 设置临时可执行启动器，启动器写 PID 后阻塞于文件门闩，释放后才 exec 真实 CLI；没有伪造协议回复。确认 PID 标记存在且新建按钮 busy 后，通过实际“注销”按钮退出。两引擎均回到登录页、启动器 PID 不再存活、会话索引没有新增记录；重新登录并成功连接真实 CLI，证明旧 open 已释放准入锁、新 lease 可正常工作。此处是受控启动延迟故障注入，不是网络握手超时或应用 Quit 验证。
- 首次执行在整页重载期间失去 WebView 脚本回传而超时；临时脚本改为订阅宿主页面加载事件后再检查新页面，并等待明确的错误密码提示，随后整组通过。未修订生产权限、账户或 Agent 生命周期实现。
- 临时验收 Rust 入口、隔离数据库和启动器已清理，原握手 smoke 保留。未调用模型生成、不修改全局 CLI 配置；未运行无关全量测试。剩余边界见下表。

### 运行、审批与关闭竞争收口（2026-11-04）

- **缺陷一与修复**：独立进程复现 `AgentManager.shutdown()` 在连接握手未完成时返回 true，但子进程仍存活、open 未完成。原因是未登记的连接不在 active 表。manager 现在为创建/恢复握手建立子 lease，同时监听全局关闭；关闭时继续等待 open 的失败清理，shutdown 限时等待 admission 后才清理活动表，无法取得 admission 返回 false。未改变账户 lease 的归属边界。修复后独立程序报告 `shutdown_result=true connecting_pid_alive=false opening_finished=true opening_rejected=true`。
- **缺陷二与修复**：完整 WKWebView 复现已有旧会话时再创建新连接，点击“停止并关闭”移除标签后，新连接仍以 idle 存活。`prepareAgentClose` 原先只在 busy 且无 session 时等待；改为 busy 时先等待，再选择最新 session 关闭。超过原有 10 秒等待期限仍保留标签。没有新增自动重试或改变普通页面卸载不停止会话的约定。
- **永久回归**：`cargo test --offline -p dgos-agents` 5/5，通过原有 2 项和新增 3 项进程 fixture 回归：创建中 shutdown、恢复中 shutdown 无迟到登记、发送尚未收到确认时 shutdown 拒绝请求并回收进程。fixture 使用真实 OS 子进程及文件门闩，不调用模型、不代表两家协议兼容证据。`npx vitest run tests/agent-close-boundaries.test.tsx tests/agent-workbench.test.tsx` 6/6，新增 2 项覆盖替换连接时等待并关闭新会话、等待超时保留旧标签。首次新增文件误用 `.test.ts` 未被仓库发现，改为 `.test.tsx` 后以上 6 项才是有效结果。
- **真实账户切换**：临时 `DGOS_LIVE_BOUNDARIES=1 cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture` 使用原生产 dist、完整外壳、真实 CLI 与模型。Codex/Claude 各在 running 和 waiting 状态通过实际账户切换 UI 切到 bob 再回 alice；旧会话读取被拒绝，真实 pending 审批的迟到响应在新账户及返回原账户后均被拒绝。四个场景通过；running 是宿主观测到运行态，不等于工具已开始产生外部副作用。该整轮随后在标签脚本失败，不能记作整轮通过。
- **实际应用退出事件**：将生产退出回调临时提取为同一个函数供验收 app 调用，通过 `AppHandle.exit(0)` 触发真实 ExitRequested。两家各测受控连接中和真实握手后 idle，共 4 次退出 0；外部检查记录 PID 均不存在。环境为 `DGOS_LIVE_BOUNDARIES=1 DGOS_QUIT_ENGINE=codex|claude`，idle 另加 `DGOS_QUIT_ACTIVE=1`，执行已编译的 `webview_smoke_agents` 二进制。临时提取已恢复原内联回调。未注册其他领域 manager，故仅覆盖 Agent 独立清理分支，不证明完整宿主 5 秒联合退出预算或原生菜单/快捷键。
- **修复后真实界面验证**：当前工作区其他改动使 `npm run build` 在 AppShell/WorkspaceTools 三处设计规则失败，随后 `npx tsc` 在 AppShell.tsx:76 的语法错误失败；本批未修改这些文件。改用临时 Vite 入口挂载生产 AgentWorkbench、SurfaceScope、TabBar、workspace controller、ModalHost 与 store，真实 WKWebView/IPC/CLI。`npx vite build --config vite.agent-boundary.config.ts` 和 `DGOS_LIVE_BOUNDARIES=1 DGOS_TABS_ONLY=1 cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture` 通过：两家连接超时关闭保留标签，已有会话时第二次连接中的关闭等待新运行完成并确认 closed。没有模拟 Agent 命令返回。此证据为实际生产组件组合，完整外壳修复后复验仍受构建阻塞。
- 临时 Rust/HTML/TSX/Vite 入口、独立复现程序、生成 bundle、启动器和数据库已清理；保留两个生产修复及五个新增回归。无全局 CLI 配置修改。未扩大 V7 开发或发布状态。

### 完整外壳与联合退出补验（2026-11-04）

- 当前工作区的 AppShell/WorkspaceTools 已由其他修改更新，上批构建阻塞不再存在。`npm run build` 完整通过：令牌、依赖、Skills、设计检查、TypeScript、桌面与 LAN Vite 产物。本批无需修改前端实现。
- 临时命令 `DGOS_JOINT_ACCEPTANCE=1 cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture` 加载本轮完整生产 dist，在可见 macOS WKWebView 中运行 App、AccountGate、工作区及 ModalHost；隔离迁移数据库与真实账户服务，注册真实 Agent IPC。Codex/Claude 各先创建 idle 会话，再通过文件门闩启动器阻塞第二次连接。UI 点击“停止并关闭”，等待 11 秒后标签仍在且新建按钮 busy；再次关闭并释放门闩，标签移除，新连接 PID 已不存在，宿主会话存在 closed 记录。启动器释放后 exec 真实 CLI，没有伪造协议或模型回复。两个引擎全部通过；旧 idle 会话按既有多会话规则留存，最终应用退出统一回收。
- 联合退出验收同时注册真实 AgentManager、McpState、WorkbenchManager、ChatManager、LanService 和 SessionResolver；LAN 通过生产 `lan::build` 实际启动私网监听，MCP monitor 已启动。临时提取生产 ExitRequested 回调供验收 app 共用，执行 `AppHandle.exit(0)`，额外断言联合返回值精确匹配 `Ok(((Ok(()), Ok(()), true), true))`。通过 `cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents --no-run` 构建后，外部 Python 驱动该二进制，以 `DGOS_JOINT_ACCEPTANCE=1`、隔离的 `DGOS_ACCEPTANCE_ROOT`、`DGOS_QUIT_ENGINE=codex|claude`，以及 idle 场景的 `DGOS_QUIT_ACTIVE=1` 执行四次。退出前通过文件屏障记录真实 PID/PGID，退出后 `kill(pid, 0)` 和 `kill(-pgid, 0)` 均报告进程不存在。
- 四次均退出 0、联合清理结果成功：Codex 连接中 0ms（毫秒取整）、idle 217ms；Claude 连接中 2ms、idle 2ms，均在生产 5 秒期限内。完整外壳场景最终联合退出另观测到 176ms。计时从 ExitRequested 回调进入开始；不等于退出前所有工作从未提前响应取消，也不证明任意负载均满足预算。
- 覆盖边界：MCP、聊天和工作台 manager 存在但无活动任务，LAN 无远程会话；未验证多个领域任务同时执行、超时故障注入、真实菜单/快捷键、辅助技术或其他平台。模块目录及无关 Provider/Skills 宿主命令未注册，因此完整前端外壳通过不等于所有模块服务集成通过。验收通过 JS 驱动实际 DOM 控件，不作为物理鼠标证据；未调用模型生成或修改全局 CLI 配置。
- 验收编译期间，另一项工作新增 UI smoke 调用时函数尚未到位，首次 `--no-run` 因 `ui_keys::shell` 缺失失败；函数随后已写入，重建通过。本批未修改该工作；其 `ui.rs:298` 不可达语句警告仍在。临时退出回调提取、断言、Rust 验收入口及隔离数据全部移除，原握手 smoke 恢复；本批未发现新的产品缺陷，无新增永久代码或测试。

### 多领域活动任务与超时补验（2026-11-04）

本批发现并修复 MCP 审计写锁阻塞子进程回收；正常联合退出、5 秒超时和新进程恢复均已验证。唯一详细证据与限制见 [V1 实现状态：多领域活动任务退出与数据库写锁](../当前版本/V1-实现状态.md#2026-11-04-多领域活动任务退出与数据库写锁)。本批 Agent 为受控 Codex 协议 fixture，不增加真实 CLI 或扩展兼容结论。

### 真实交互与 Claude 扩展对照（2026-11-04）

Codex/Claude 的真实文件任务、中断、继续、关闭/恢复，以及 Claude 审批、问答、Skill/Hook/MCP、显式插件调用和 Haiku 子任务已有定向证据。插件斜杠命令首轮偏离在桌面和独立 CLI 均复现；不宣称已修复。唯一详细记录见 [V1 实现状态：Agent 真实交互与 Claude 扩展对照](../当前版本/V1-实现状态.md#2026-11-04-agent-真实交互与-claude-扩展对照)。

## 8. 后续任务顺序

以下为 D127 独立会话切片的后续任务，不扩展为 V7 整体开发授权。

| 顺序 | 任务 | 完成判定与前置条件 |
| --- | --- | --- |
| 1 | 工作区剩余边界 | 完整外壳、管理器共存、活动任务退出及真实模型交互已有分层证据；原生点击与键盘事件仅覆盖指定控件，仍需完整物理输入和辅助技术覆盖 |
| 2 | 原生扩展组合兼容性 | Claude 桌面 Skill/Hook/本地 MCP 与显式加载插件已有对照；自动安装及任意插件组合未证明，保持 DGOS Skills/MCP 不自动同步边界 |
| 3 | 首轮与辅助模型兼容问题 | 当前 Haiku 子任务成功；插件斜杠命令首轮在桌面与独立 CLI 同样偏离，显式 Skill 调用成功；仍需服务/模型侧定位，不宣称首轮问题已修复 |
| 4 | 分发与发布门禁 | macOS 本地包构建与摘要门禁、移位 release 会话闭环、缺失依赖类别及安装版账户/Agent 主流程已有验证，见[本机分发证据](../当前版本/V1-实现状态.md#2026-11-04-agent-本机分发验证)；直接启动需使用无符号链接的真实路径，常规启动使用 LaunchServices；仍缺签名公证、跨平台及完整交互矩阵，第三方认证遵守 §2 |

联合退出和真实交互证据已补齐至上述范围；后续按表内剩余边界推进，不改变 V7 整体门禁，也不宣称全部上游扩展兼容。
