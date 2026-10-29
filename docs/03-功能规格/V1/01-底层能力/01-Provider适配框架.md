---
title: "Provider 适配框架"
version: V1
feature_id: V1-FR-001
domain: 底层能力
spec_status: Ready
risk: 高风险
track: Provider 能力
updated: 2026-10-29
---
# V1-FR-001：Provider 适配框架

> 三方兼容协议扩展（2026-10-29，草案）：V1 支持用户接入自备的三方服务，只要兼容 OpenAI Chat Completions 或 Anthropic Messages 即可（D-待编号-A）。用户选择厂商（首批 OpenAI、Anthropic、自定义）并确定适配器、填写 base_url 和 API Key，拉取模型列表，宿主自动分类，保存后即可调用；模型在哪些应用场景使用、如何限定，留给后续的场景绑定设计，不在本文范围。既有 OpenAI 兼容实现的缺口一并补齐（§2.4.7）。协议适配层方案见 [ADR-0003](../../../06-决策记录/ADR/0003-Provider协议适配层与规范化消息模型.md)（Proposed），具体口径待 D-待编号-B 冻结。两条决策尚未写入冻结决策文档（另一项 FR-004 工作正在并行追加，编号在其结束后顺延登记），占位编号不是正式编号。扩展内容集中在 §2.4、§3.5～§3.7、§4.3、AC15～AC37、§6.3、§8.1 和[技术设计](01-Provider适配框架-技术设计.md)。AC01～AC14 原文与编号不变，按原口径收口，不被扩展范围阻塞；AC15～AC37 在两条决策冻结、§2.4.6 待确认项关闭之前是草案，不作为编码输入。
>
> D023 桌面补充基线（2026-10-28）：宿主派生用户身份，Provider DTO 不携带 secret，凭据保存到 OS vault，历史明文字段通过可重试迁移清空。当前字段与迁移规则见 [接口契约 §4.0](../../../04-技术架构/当前版本/V1-接口契约.md#40-当前桌面实施契约d023)、[数据模型 §4.2](../../../04-技术架构/当前版本/V1-数据模型.md#42-provider-管理)；本文早期 AES/SQLCipher 表草图不再作为当前凭据实施依据。前置工作包与证据见 [FR-004 前置收口设计](../../../04-技术架构/当前版本/V1-FR004前置收口设计.md)，不改变既有 AC 编号。§6 中带 `user_id` 的命令草案及 `stream_provider` 均为早期草案；当前已注册命令见 `src-tauri/src/lib.rs`、`src-tauri/src/commands/provider_commands.rs` 和接口契约 §4.0。

## 1. 功能元信息

| 项目 | 内容 |
| --- | --- |
| 编号 | V1-FR-001 |
| 功能名称 | Provider 适配框架 |
| 所属版本 | V1 |
| 领域 | 底层能力 |
| 规格状态 | Ready |
| 优先级 | P0（必须） |
| 风险等级 | 高风险（外部调用、凭据、并发流式调用、缓存代次；多协议扩展增加协议转换与厂商差异） |
| 上游依赖 | 无（基础能力） |
| 下游影响 | V1-FR-004（模块 `providers.call`）、V1-FR-006（AI 工作台）、V1-FR-007（Provider 设置界面）、V2 画布系统 |
| 规格负责人 | TBD |
| 实现状态 | 见 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)，本文不记录 |

## 2. 功能概述

### 2.1 一句话描述

统一适配各厂商 AI 模型 API，提供模型列表获取、分类标签、同步/流式调用能力，支持多 Provider 配置、排序和手动选择。

### 2.2 详细说明

Provider 适配框架是 DGOS 的底层核心能力，负责屏蔽各个 AI 模型提供商（OpenAI、Claude、通义千问等）的 API 差异，为上层应用提供统一的调用接口。

**核心价值**：
- ✅ 统一接口：应用层不需要关心具体 Provider 的 API 细节
- ✅ 多 Provider 支持：用户可以配置多个同类型 Provider（如多个 OpenAI 账号）
- ✅ 模型分类：自动给模型打标签（text-generation、image-generation 等）
- ✅ 灵活选择：应用层可以手动选择 Provider，或使用默认排序

**用户价值**：
- 开发者：只需要调用统一接口，不用处理各厂商 API 差异
- 最终用户：可以配置多个账号，灵活切换和管理

### 2.3 功能边界

**包含**：
- ✅ Provider CRUD 管理（增删改查）
- ✅ 统一的 Provider trait 接口定义
- ✅ OpenAI 兼容适配器实现（原“V1 唯一实现”的口径由 D-待编号-A 修订：V1 另加 Anthropic 协议适配器，并允许接入任意三方兼容服务，见 §2.4）
- ✅ 模型列表获取与缓存（永久缓存，手动刷新）
- ✅ 模型分类标签（内置维护：text-generation、image-generation、video-generation、audio-generation）
- ✅ 同步调用接口
- ✅ 流式调用接口（SSE）
- ✅ 使用记录保存
- ✅ 多 Provider 排序机制（用户拖拽）
- ✅ Provider 启用/禁用开关
- ✅ 并发调用支持（不排队）
- ✅ 可配置重试策略（应用层配置）

**不包含**（明确说明原因）：
- ❌ 其他厂商适配器（Claude、通义千问等）→ V4 云端增强时实现（已由 D-待编号-A 修订：Anthropic 协议与三方兼容服务接入前移到 V1，见 §2.4；V4 只保留官方 Provider、积分与计费）
- ❌ 图片输入、提示缓存与缓存用量字段、Embeddings、图片生成、音频、OpenAI Responses API、Batch → 路线图未分配（D-待编号-A）
- ❌ 自动切换到备用 Provider → 应用层自己决定策略
- ❌ 连接池管理 → V1 简化实现，直接调用
- ❌ 流式输出断点续传 → 复杂度高，V2 考虑
- ❌ Provider 健康检查 → V2 监控功能
- ❌ 请求排队机制 → 不需要，允许并发

### 2.4 三方兼容协议扩展（草案）

来源：D-待编号-A（用户选定的范围）、D-待编号-B（协议适配层口径，草案）、[ADR-0003](../../../06-决策记录/ADR/0003-Provider协议适配层与规范化消息模型.md)。实现方式见[技术设计](01-Provider适配框架-技术设计.md)。

#### 2.4.1 范围

目标：用户接入自备的三方服务，只要它兼容 OpenAI Chat Completions 或 Anthropic Messages，就能配置、拉取模型、自动分类并调用。

| 项 | V1 范围 |
| --- | --- |
| 用户输入 | 厂商（首批 OpenAI、Anthropic、自定义；厂商有多个端点时再选适配器）、名称、base_url（预填端点默认值，可修改）、API Key。高级选项（默认折叠）：鉴权头方式、允许明文 HTTP。可手动添加模型 |
| 适配器（`provider_type`，由所选厂商的端点决定） | 只有 `openai_compatible`（OpenAI Chat Completions）和 `anthropic`（Anthropic Messages）。提供 Anthropic 兼容端点的第三方服务也使用 `anthropic` |
| 基线能力 | 文本对话（同步与流式）、工具调用（D022 工作台工具循环使用） |
| 附加能力 | 推理内容：设置推理档位，返回可展示的推理文本，工具循环中按上游行为回传；结构化输出：按调用方给出的 JSON Schema 输出并由宿主校验，V1 只用于非流式调用 |
| 本功能输出的数据 | Provider、模型列表、每个模型的分类（kind）与能力提示、调用能力（同步、流式、取消） |
| 不在本功能范围 | 模型在哪些应用场景可用、如何限定、统一模型选择器：留给后续的场景绑定设计，本功能只保证数据足够支撑它（§2.4.3）；§2.3“不包含”所列的其他端点与能力 |

#### 2.4.2 厂商与兼容画像

用户添加 Provider 时先选择**厂商**。厂商是编译进宿主、带版本的数据条目，下面有一个或多个**端点**，每个端点对应一个适配器（`provider_type`）、一个默认 base_url、官方主机和官方画像。厂商只有一个端点时，选厂商就确定了适配器；有多个端点时，用户再选端点（例如“OpenAI 兼容”或“Anthropic 兼容”）。用户随后填写名称和 API Key，base_url 可以修改。

| 厂商 | `vendor_id` | 端点：适配器（`provider_type`） | 默认 base_url | 官方主机 |
| --- | --- | --- | --- | --- |
| OpenAI | `openai` | `openai_compatible` | `https://api.openai.com/v1` | `api.openai.com` |
| Anthropic | `anthropic` | `anthropic` | `https://api.anthropic.com` | `api.anthropic.com` |
| 自定义 | `custom` | `openai_compatible` 和 `anthropic`（两个端点） | 无，用户填写 | 无（始终使用通用画像） |

- 首批有 OpenAI、Anthropic 和“自定义”三个厂商（§2.4.6 第 13 项）。接入没有内置条目的三方兼容服务（Ollama、LM Studio、vLLM、各类网关、DeepSeek 等）时选“自定义”，再选 OpenAI 兼容或 Anthropic 兼容，并填写 base_url。用 OpenAI 厂商改写 base_url 去连其他服务不会报错，但走通用画像，界面不推荐这样做。
- 新增厂商（候选见技术设计，建议第一个加入 DeepSeek，它同时提供 OpenAI 兼容和 Anthropic 兼容两个端点；其余如阿里云百炼、Moonshot、智谱、OpenRouter、硅基流动、Ollama、LM Studio）只往厂商表加一行及其端点，复用已有适配器；只有出现新的传输协议才新增 `provider_type` 和适配器（ADR-0003）。加入前需要用真实 Key 验证该厂商的画像（§2.4.6 第 15 项）。
- Provider 的 `config.vendor_id` 记录所选厂商，`provider_type` 记录所选端点的适配器。缺省 `vendor_id` 时按 `provider_type` 和 base_url 推断：主机属于某厂商该适配器端点的官方主机则为该厂商，否则为 `custom`；既有数据不需要迁移，调用行为也不变（画像本来就按主机选择）。宿主保存前校验 `vendor_id` 存在且该厂商有 `provider_type` 对应的端点，否则返回 `unsupported_provider_type`。
- 厂商表只在宿主维护，界面通过只读命令 `list_provider_vendors` 读取，前端不自己维护厂商列表（共享类型只保留一个权威定义）。

厂商声称兼容的服务，细节仍常有差异（鉴权头、参数名、是否有模型列表、推理字段名）。宿主用**兼容画像**处理这些差异：

- 画像是端点下的数据，描述请求的形状。每次调用时，base_url 的主机属于该端点的官方主机，就使用该端点的**官方画像**；否则使用该适配器的**通用画像**（`custom` 厂商没有官方主机，始终如此）。base_url 变化后画像随之重新匹配。
- 官方主机按完整域名标签匹配（子域名必须在 `.` 边界），`api.openai.com.evil.example` 不会被当作官方主机。
- 画像字段：鉴权方式、协议版本头、模型列表端点与分页方式、`max_tokens` 参数名、是否发送 `stream_options`、`tool_choice`、`parallel_tool_calls`、推理请求参数模板、推理返回字段名、结构化输出方式、是否拒绝采样参数。每条官方画像登记官方文档来源与核实日期。
- 画像只能改变请求的形状，不能改变目标主机，不能放宽 HTTPS 策略，也不能接触凭据。
- 通用画像取保守默认：只发送协议的标准字段，不主动发送厂商专属参数；解析响应时宽容（§2.4.7）。
- 用户可在高级选项里覆盖两项：`auth_style`（`bearer`、`x-api-key`、`both`）和 `allow_insecure_http`，保存到 `config`。宿主保存前校验这两项的取值；`config` 中现有的重试配置不受影响。
- 修正官方画像或通用画像只改画像数据，不改适配器代码。首批画像见技术设计“厂商表与兼容画像”。

#### 2.4.3 模型自动分类

拉取模型列表时，宿主为每个模型计算分类 `kind`，用户不需要操作。分类由宿主计算，不接受前端提交的分类、能力或模型主键（见 §2.4.4）。

| `kind` | 含义 |
| --- | --- |
| `text-generation` | 对话、补全（含多模态输入的对话模型） |
| `embedding` | 向量嵌入 |
| `rerank` | 重排序 |
| `image-generation` | 图片生成 |
| `video-generation` | 视频生成 |
| `speech-synthesis` | 语音合成（文本转语音） |
| `speech-transcription` | 语音转写（语音转文本） |
| `moderation` | 内容审核 |

- `anthropic` 协议的模型一律是 `text-generation`（协议本身就是对话接口），`kind_source=protocol`。
- `openai_compatible` 协议的模型按规范化后的模型 ID 匹配内置规则（去掉 `vendor/` 之类的路径前缀，忽略大小写）。命中规则的 `kind_source=rule`；全部规则都没命中时归为 `text-generation`，`kind_source=default`。理由：三方网关列出的模型绝大多数是对话模型，名称无法穷举，默认归为对话比默认隐藏更符合用户预期；代价是名称特殊的非对话模型可能被误归为对话。
- 规则表和用例见技术设计“模型分类规则”，每次增补规则必须同时增补用例。
- 本规则细化 D014：原标签 `audio-generation` 把语音合成和语音转写混为一类（如 `whisper` 是转写，不是生成），拆为两类；新增 `embedding`、`rerank`、`moderation`。D014 的“标签由内置规则维护”不变，用户不能给远程模型自定义分类。手动添加模型时用户可以选择 `kind`（§2.4.4）。
- 能力提示（流式、视觉、工具、推理、结构化输出）各为 `supported`、`unsupported` 或 `unknown`，来源是上游文档化字段和 ID 规则。**能力提示只供展示和后续场景设计使用，宿主不据此拦截任何请求**；上游不支持时，错误按 §8 归一后返回。
- 上下文窗口和最大输出长度只取上游文档化的字段（例如 Anthropic `/v1/models` 的 `max_input_tokens`、`max_tokens`），取不到为空。上游元数据只提供这类数值和能力提示，不改变 `kind`；这是对 D014“不自动提取”的窄化修订，待确认（§2.4.6 第 9 项）。

#### 2.4.4 模型拉取、保存与手动模型

- 拉取：宿主请求模型列表并分类，返回预览，不写库。分页取完整（Anthropic 用 `after_id`，OpenAI 兼容服务单次返回）。
- 保存：前端提交的模型列表只有 `model_id` 和 `model_name` 被采用；`kind`、能力、主键、上下文长度一律由宿主重新计算。伪造的分类不会入库。
- 手动模型：用于没有模型列表端点的服务和本地模型。用户填写模型 ID，可选 `kind`（默认按规则），保存为 `source=manual`。同一 Provider 下已有同 ID 模型时不重复插入。
- 刷新：只替换 `source=remote` 的行，不删除手动模型；与手动模型同 ID 的远程项跳过。
- 选择与限定：模型在哪些应用场景使用、如何限定、统一模型选择器，留给后续的场景绑定设计。本功能只保证每个模型带有 `kind`、能力提示和来源。排序第一的 Provider 作为默认值（D012）和“失败不自动切换”（D023）不变。

#### 2.4.5 连接测试

- “测试连接”先请求模型列表。画像确认没有模型列表端点时直接走探测。
- 模型列表返回 404 或 405 时，提示“该服务没有模型列表接口”，改为要求填写一个模型名，宿主向该模型发一次 `max_tokens=16` 的探测请求，任何 2xx 响应都算成功。界面在按钮旁说明这会消耗少量 token。探测成功后可以保存，模型靠手动添加。
- 两种方式只验证配置，不缓存模型列表；对未保存配置的探测不写使用记录（尚无 Provider ID）。
- 失败时给出可操作的诊断，不只返回“连接失败”：

| 现象 | 返回 | 界面提示 |
| --- | --- | --- |
| 401 | `invalid_api_key` | API Key 无效 |
| 403 | `provider_permission_denied` | Key 无权限或地区受限 |
| 200 但响应不是预期的 JSON（例如 HTML 页面） | `provider_response_invalid` | 检查 base_url 是否缺少 `/v1`，或填成了网页地址 |
| 3xx | `provider_response_invalid` | 宿主不跟随重定向，请填写最终地址 |
| 连接失败、证书错误、超时 | 原有网络错误或 `timeout` | 检查地址、网络或证书 |
| 404 或 405 | 转探测流程 | 该服务没有模型列表接口 |

#### 2.4.6 待确认项

以下默认值写入 AC 与技术设计，规格评审时由用户确认或修改。确认前 AC15～AC37 保持草案。其中第 3、4、6、7、10 项的具体数值是我的推断，不是官方文档给出的值。

| # | 事项 | 默认方案 | 备选 |
| --- | --- | --- | --- |
| 1 | Anthropic 协议下出现在非开头位置的 system 消息 | 本地拒绝，返回 `request_not_supported` | 按出现顺序合并进顶层 `system` |
| 2 | Anthropic 协议的 base_url 约定 | 不含 `/v1`，与官方 SDK 一致（2026-10-29 核对的兼容端点都按此约定）；用户粘贴的地址以 `/v1` 结尾时，宿主保存前去掉并在表单中显示规范化后的值 | 拒绝保存，提示用户修改 |
| 3 | 超时数值 | 连接 10 秒；模型列表 30 秒；非流式总期限 120 秒（与 D051 一致）；流式空闲 120 秒，不设流式总期限；响应体上限 8 MiB，单个 SSE 事件上限 1 MiB，超限按 `provider_response_invalid` 或 `stream_interrupted` 处理 | 按 Provider 配置覆盖 |
| 4 | 通用画像的鉴权 | `openai_compatible`：Bearer。`anthropic`：同时发送 `Authorization: Bearer` 和 `x-api-key`（三方对两种头的接受情况不一致，同时发送兼容面最大，两个头同等脱敏）；官方 Anthropic 画像只发 Bearer；用户可用 `auth_style` 覆盖 | `anthropic` 通用画像只发 `x-api-key` |
| 5 | 探测连接消耗少量 token | 接受，界面提示，不写使用记录 | 只允许有模型列表端点的服务做连接测试 |
| 6 | 推理档位与映射 | 规范档位 `none`、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`，宿主原样交给上游，不在本地校验子集（上游拒绝时按错误归一返回）。`anthropic` 通用画像用 `thinking: {type: enabled, budget_tokens}`，`low`/`medium`/`high` 映射为 2048/8192/16384，其他档位本地拒绝；官方 Anthropic 画像用 `adaptive` 加 `output_config.effort`。数值是推断，需 smoke 校准 | 只支持 `low`、`medium`、`high` 三档 |
| 7 | `anthropic-version` | 官方画像固定 `2026-10-21`（该版本在所有模型上拒绝 `temperature`、`top_p`、`top_k`，官方画像下带这些参数的请求本地拒绝）；通用画像固定 `2023-06-01`，三方网关对它的接受度最高（未用真实服务验证） | 所有画像固定 `2023-06-01` |
| 8 | Anthropic 工作区头 | `anthropic-workspace-id` 作为可选的非敏感配置项，只在官方画像下发送 | 不支持 |
| 9 | 上游模型元数据 | 窄化修订 D014：只取上下文窗口、最大输出长度和能力提示，且只取协议文档化的字段，不改变 `kind` | 保持 D014 原文，不读上游元数据，数值字段留空 |
| 10 | 明文 HTTP | 回环地址和私网字面 IP 允许，界面显示明文传输提示；其他主机需要用户在高级选项里勾选 `allow_insecure_http` 并看到风险说明；保存和每次调用都检查 | 只允许回环地址 |
| 11 | 参数降级重试 | 上游以 400 拒绝且错误信息点名下列参数之一时，去掉该参数重试一次，不持久化：`stream_options`、`parallel_tool_calls`、`tool_choice`、`max_tokens`/`max_completion_tokens` 互换。每次调用最多降级一次，不记录上游错误正文 | 不降级，直接返回 `provider_request_rejected` |
| 12 | 用户修正远程模型的分类 | V1 不支持（D014）；只有手动添加的模型可以选 `kind` | V1 支持用户覆盖，需修订 D014 |
| 13 | 首批厂商与“自定义”厂商 | 首批为 OpenAI、Anthropic（用户指定）加“自定义”（用户确认）；“自定义”有 OpenAI 兼容和 Anthropic 兼容两个端点，没有默认 base_url，始终使用通用画像；厂商下是端点列表，端点才对应适配器 | 不设“自定义”，用 OpenAI 或 Anthropic 厂商改写 base_url 接入（已被取代） |
| 14 | 官方画像的生效范围 | 只在 base_url 的主机属于该端点的官方主机时生效，其余主机用通用画像。以后新增的厂商是否沿用这一规则，在新增时逐个确定 | 选了厂商就始终使用其画像，不看主机 |
| 15 | 第一个扩展厂商 | 建议 DeepSeek（OpenAI 兼容和 Anthropic 兼容两个端点）。它的画像来自官方文档页面，没有用真实 Key 验证过，所以不放进首批；验证通过后再加入，加入前用“自定义”接入 | 首批就加入 DeepSeek |

#### 2.4.7 既有 OpenAI 兼容实现的补齐

下表是 2026-10-29 阅读代码发现的缺口，没有用测试复现。它们对三方兼容服务影响最大，一并在本功能内补齐；状态与证据只回写到 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)。

| # | 缺口（代码位置） | 影响 | 处理 |
| --- | --- | --- | --- |
| 1 | 模型分类算出后没有保存：适配器把 `model_type` 写死为 `chat`，`supports_streaming`、`supports_vision` 从不为真（`adapters/openai.rs` 的 `list_models`）；仓库读取时又把 `model_type` 写死为 `chat`、`max_output_tokens` 为空（`model_repository.rs`） | AC04 只在函数层面成立，自动分类对用户不可见，后续场景无数据可用 | AC27、B01-45 |
| 2 | 分类函数默认全部归为文本生成，`embedding`、`rerank`、视频等被当作对话模型；`whisper` 与 `tts` 混为 `audio-generation`（`classify_model`） | 三方网关的模型列表被误分 | AC27、B01-45 |
| 3 | `list_models` 与 `test_connection` 不把 401 映射为 `invalid_api_key`；`test_connection` 对非 2xx 返回 `Ok(false)`，创建时被报成 `provider_unavailable`（`manager.rs` 的 `create_provider`） | 与 AC02 要求的“API Key 无效”不符 | AC37、B01-54 |
| 4 | 响应结构要求 `usage` 三个字段齐全（`OpenAIChatResponse`）；缺字段时反序列化失败，被当作网络错误，再被重试 | 不返回用量的三方服务无法使用，且被重试三次后报错 | AC28、B01-69 |
| 5 | 重试策略对除 401 与 `ProviderNotFound` 外的所有错误重试，包括 400、404 和 JSON 解析失败（`retry.rs`） | base_url 填错时重试三次才报错；不可重试错误被改写为 `max_retries_exceeded` | AC30、B01-55～56 |
| 6 | 流式按网络包解析，每包只取第一个 `data:` 行，跨包事件不缓冲，解析失败产生空块（`openai.rs` 的 `stream`） | 流式输出丢字 | AC21、B01-59 |
| 7 | 客户端总超时 60 秒同样作用于流式响应体 | 超过 60 秒的流式输出被截断 | AC31、B01-57～58 |
| 8 | 流式调用不写使用记录，请求也没有 `stream_options` | AC14 对流式不成立 | AC19、AC21、B01-63 |
| 9 | 工具调用请求固定带 `tool_choice` 和 `parallel_tool_calls`；`max_tokens` 固定使用旧字段名 | 部分三方服务和 OpenAI 推理型模型会返回 400 | AC36、B01-71 |
| 10 | 客户端使用默认的重定向策略；没有检查 URL scheme | 重定向后请求可能到达另一个主机；`file://` 一类地址未被拒绝；FR-001 §9.2 的“强制 HTTPS”没有实现 | AC32、AC34、B01-37、B01-68 |
| 11 | `create_provider_with_models` 把前端提交的模型数据原样入库，包括主键和能力（`provider_repository.rs` 的 `create_with_models`） | 违反“身份和能力由宿主派生”（AGENTS.md §3）；后续场景若依赖分类，前端可以伪造 | AC35、B01-67 |
| 12 | 创建 Provider 必须模型列表接口成功（`create_provider`） | 没有模型列表接口的服务无法添加 | AC17、B01-39～40 |
| 13 | 响应体无大小上限（工具调用路径除外）；模型列表不去重、不限量 | 恶意或出错的端点可以耗尽内存 | AC28、B01-70 |
| 14 | 桌面端没有注册流式命令（接口契约 §4.0 已注明） | AC06 无法端到端验证，后续场景无法流式调用 | AC33、B01-64～65 |
| 15 | 前端表单只有一个写死的 “OpenAI Compatible” 选项，类型值和默认地址写在组件里（`ProviderForm.tsx`、`ProviderList.tsx`） | 无法添加 Anthropic Provider，也无法扩展厂商；Ollama、DeepSeek 等只能借用 OpenAI 选项改地址 | AC16、B01-36 |

**宽容读取规则**（适用于两种协议的所有响应，B01-69）：忽略未知字段；可选字段缺失时取默认值；`usage` 缺失记为 0 并保留“用量未知”的内部标记；`choices` 或内容块为空时 `content` 为空串；未知的结束原因按 `stop` 处理并记录类别名；推理文本从已知字段名（`reasoning_content`、`reasoning`、`reasoning_details`）中读取，命中哪个就记录哪个；模型列表缺少 `id` 的项跳过，重复 `id` 只保留第一项，`id` 超过 255 字符的项跳过，总数超过上限时截断。响应不是预期的 JSON、响应体超限，或收到 3xx，统一为 `provider_response_invalid`，不重试（B01-70）。

## 3. 核心流程

### 3.1 配置 Provider 流程

```
用户打开系统设置
  ↓
点击"Provider 管理"
  ↓
点击"添加 Provider"
  ↓
选择类型：OpenAI 兼容
  ↓
填写配置：
  - 名称：我的 OpenAI
  - Base URL：https://api.openai.com/v1
  - API Key：sk-xxx
  ↓
点击"测试连接"
  ↓
系统调用 Provider API 验证
  ↓
测试成功 → 保存配置
  ↓
自动获取模型列表（首次）
  ↓
模型列表缓存到本地
  ↓
显示在 Provider 列表中
```

### 3.2 调用 Provider 流程

```
应用层（AI 工作台）发起调用
  ↓
选择 Provider（或使用默认排序第一个）
  ↓
选择模型（从缓存的模型列表）
  ↓
构造请求参数（prompt、temperature 等）
  ↓
调用 Provider 适配器
  ↓
适配器转换为 Provider 原生格式
  ↓
发送 HTTP 请求到 Provider API
  ↓
（如果失败）检查重试配置
  ↓
（如果配置了重试）按指数退避重试
  ↓
（如果是流式）逐块返回结果
  ↓
（如果流式中断）保存已接收部分
  ↓
记录使用日志
  ↓
返回结果给应用层
```

### 3.3 刷新模型列表流程

```
用户在 Provider 管理界面
  ↓
选择某个 Provider
  ↓
点击"刷新模型列表"
  ↓
系统调用 Provider API 获取最新模型
  ↓
更新本地缓存
  ↓
重新给模型打分类标签
  ↓
显示"已更新 X 个模型"
```

### 3.4 多 Provider 排序流程

```
用户在 Provider 管理界面
  ↓
看到所有已配置的 Provider 列表
  ↓
拖拽某个 Provider 调整顺序
  ↓
系统保存新的排序
  ↓
应用层使用时，默认选择排序第一个
```

### 3.5 添加三方兼容 Provider 流程（三方兼容协议扩展，草案）

```
用户点击"添加 Provider"
  ↓
选择厂商（首批：OpenAI、Anthropic、自定义）
  → 厂商有多个端点时再选适配器（“自定义”：OpenAI 兼容 / Anthropic 兼容）
  → 预填端点的默认 base_url（“自定义”没有默认值，必须填写）
填写名称、API Key；base_url 可修改
  （本地服务不校验 Key，但 D023 要求非空，可填任意占位值）
  （高级选项默认折叠：鉴权头方式、允许明文 HTTP）
  ↓
宿主校验：vendor_id 存在且该厂商有 provider_type 对应的端点；scheme 与明文 HTTP 策略；base_url 规范化
  ↓
点击"测试连接"
  ├─ 请求模型列表成功 → 连接成功
  ├─ 401 / 403 / 响应不是 JSON / 3xx → 显示 §2.4.5 的诊断提示
  └─ 404 或 405 → 提示"该服务没有模型列表接口" → 填写模型名 → 探测请求
  ↓
点击"拉取模型"（有模型列表接口时）
  → 宿主取完整列表并自动分类 → 界面预览 kind 与能力提示
  ↓
保存：宿主重新计算每个模型的分类与能力（不采用前端提交的值）
  → Provider、凭据引用和模型在一个事务内提交（D023）
  ↓
之后可手动添加模型、刷新模型列表
```

### 3.6 手动模型流程（多协议扩展，草案）

```
用户在 Provider 详情中点击"添加模型"
  ↓
输入模型 ID，可选择分类 kind（默认按规则自动判断）
  ↓
保存为 source=manual；同一 Provider 下已有同 ID 模型时不重复插入
  ↓
之后"刷新模型列表"只替换 source=remote 的行，手动模型保留
```

### 3.7 保存后的使用（范围说明）

保存后的 Provider 和模型已带有 `kind`、能力提示和来源，可被应用层按 Provider 与模型调用（§3.2）。哪些应用场景可以使用哪些模型、如何限定、是否提供统一的模型选择器，由后续的场景绑定设计决定，本功能不判断场景。调用本身不因 `kind` 或能力提示被宿主拦截；上游不接受时按 §8 归一返回错误。

## 4. 数据模型

### 4.1 涉及的数据表

**providers 表**（核心）：
```sql
CREATE TABLE providers (
    id TEXT PRIMARY KEY,              -- UUID
    name TEXT NOT NULL,               -- 用户自定义名称，如"我的 OpenAI"
    provider_type TEXT NOT NULL,      -- 类型：openai_compatible, claude, qwen 等
    base_url TEXT NOT NULL,           -- API Base URL
    api_key TEXT NOT NULL,            -- 加密存储的 API Key
    enabled BOOLEAN DEFAULT TRUE,     -- 是否启用
    sort_order INTEGER DEFAULT 0,     -- 排序权重（越小越靠前）
    config JSON,                      -- 额外配置（重试次数等）
    created_at TIMESTAMP,
    updated_at TIMESTAMP,
    user_id TEXT,                     -- 所属用户
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

**models 表**（缓存）：
```sql
CREATE TABLE models (
    id TEXT PRIMARY KEY,              -- UUID
    provider_id TEXT NOT NULL,        -- 所属 Provider
    model_id TEXT NOT NULL,           -- 模型 ID，如 gpt-4
    model_name TEXT,                  -- 模型显示名称
    capabilities JSON,                -- 能力标签数组：["text-generation"]
    context_length INTEGER,           -- 上下文长度
    is_available BOOLEAN DEFAULT TRUE,-- 是否可用
    synced_at TIMESTAMP,              -- 同步时间
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(provider_id, model_id)
);
```

**provider_usage_logs 表**（使用记录）：
```sql
CREATE TABLE provider_usage_logs (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    request_tokens INTEGER,
    response_tokens INTEGER,
    total_tokens INTEGER,
    latency_ms INTEGER,              -- 响应时间（毫秒）
    success BOOLEAN,
    error_code TEXT,                 -- 如果失败，错误码
    created_at TIMESTAMP,
    FOREIGN KEY (provider_id) REFERENCES providers(id),
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

### 4.2 读写关系

**读操作**：
- 获取 Provider 列表：`SELECT * FROM providers WHERE user_id = ? ORDER BY sort_order`
- 获取模型列表：`SELECT * FROM models WHERE provider_id = ? AND is_available = TRUE`
- 查询使用记录：`SELECT * FROM provider_usage_logs WHERE user_id = ? ORDER BY created_at DESC`

**写操作**：
- 创建 Provider：`INSERT INTO providers`
- 更新 Provider：`UPDATE providers SET ... WHERE id = ?`
- 删除 Provider：`DELETE FROM providers WHERE id = ?`（级联删除 models）
- 缓存模型列表：`INSERT OR REPLACE INTO models`
- 记录使用日志：`INSERT INTO provider_usage_logs`

### 4.3 多协议扩展的数据变化（草案）

物理结构以 migration 为准；以下在 D-待编号-B 冻结后同步到 [V1 数据模型](../../../04-技术架构/当前版本/V1-数据模型.md)。

| 对象 | 变化 | 兼容性 |
| --- | --- | --- |
| `models.source` | 新增列，`remote` 或 `manual`，默认 `remote`；追加 migration | 旧行升级后为 `remote`；forward-only，不修改已执行的 migration |
| `models.kind` | 新增可空列，取值见 §2.4.3；追加 migration | 旧行为 NULL：读取时按 `model_id` 规则即时计算，不回写；下一次刷新或保存时持久化 |
| `models.kind_source` | 新增可空列，`protocol`、`rule`、`default` 或 `manual` | 同上 |
| `models.max_output_tokens` | 新增可空整数列（现有表没有这一列，DTO 字段一直为空） | 旧行为 NULL |
| `models.capabilities` | JSON 形状从能力数组扩展为对象 `{streaming, vision, tools, reasoning, structured_output}`，每项为 `supported`、`unsupported` 或 `unknown` | 读取时兼容旧数组（数组里的 `streaming`、`vision` 视为 `supported`，其余为 `unknown`），不需要 migration |
| `providers.config` | JSON 中增加 `vendor_id`、`auth_style`、`allow_insecure_http`，与现有重试配置并存 | `vendor_id` 缺省时按 `provider_type` 和 base_url 推断（官方主机归对应厂商，否则 `custom`）；`auth_style` 缺省为画像默认值，`allow_insecure_http` 缺省为 `false`；不需要 migration。已知细节：现有代码把整个 `config` 反序列化为重试配置，缺少重试三项时解析失败并回落到默认重试（`manager.rs` 的 `create_adapter`），新增键不改变这一行为 |
| `provider_usage_logs` | 不变 | 两种协议都把推理 token 计入输出 token；Anthropic 不返回总数，由宿主计算输入加输出；缓存 token 不在范围内；上游不返回用量时记 0（没有“未知”标记列，这是已知限制） |

分类是 `(协议, model_id)` 的函数，仍然持久化，原因有两个：手动模型的 `kind` 由用户指定，规则表以后更新时已保存模型的分类不应悄悄变化（沿用“永久缓存，手动刷新”）。

推理文本和思考块（含签名）不写入 SQLite、使用记录、审计或日志，只在同一工具循环的内存上下文中传递。工作台是否持久化可展示的推理文本由 FR-006 决定。

## 5. 验收标准

### 5.1 AC 逐项测试设计表

#### AC01 Provider 正常创建

Given 用户已登录

When 创建 OpenAI Provider 配置

Then Provider 保存成功，模型列表自动获取

#### AC02 API Key 无效检测

Given 用户输入错误的 API Key

When 点击"测试连接"

Then 返回 401 错误，提示"API Key 无效"

#### AC03 模型列表获取与缓存

Given Provider 已配置

When 首次获取模型列表

Then 调用 Provider API，缓存到 models 表

#### AC04 模型分类标签正确

Given 模型列表已获取

When 查询模型

Then gpt-4 标签为 text-generation，dall-e-3 为 image-generation

#### AC05 同步调用成功

Given Provider 和模型已配置

When 调用 chat.completions

Then 返回完整响应，记录使用日志

#### AC06 流式调用成功

Given Provider 和模型已配置

When 调用 chat.completions（stream=true）

Then 返回 SSE 流，逐块接收

#### AC07 流式输出中断处理

Given 流式调用进行中

When 网络中断或 Provider 断开

Then 保存已接收部分，返回错误码

#### AC08 并发调用支持

Given 多个应用同时调用

When 5 个并发请求同时发起

Then 所有请求都能成功处理，无排队

#### AC09 可配置重试策略

Given Provider 配置了 retry_count=3

When API 调用返回 500

Then 自动重试 3 次，间隔 1s/2s/4s

#### AC10 重试失败后返回错误

Given Provider 配置了 retry_count=3

When API 调用 3 次都失败

Then 返回错误，不再重试

#### AC11 多 Provider 排序

Given 用户配置了 3 个 Provider

When 拖拽调整排序

Then 排序保存，默认使用排序第一个

#### AC12 Provider 启用/禁用

Given Provider 已配置

When 禁用某个 Provider

Then 该 Provider 不出现在可选列表

#### AC13 手动刷新模型列表

Given 模型列表已缓存

When 点击"刷新模型列表"

Then 重新获取，更新缓存

#### AC14 使用记录保存

Given 调用成功

When 查询使用记录

Then 包含 tokens、延迟、时间

AC15～AC37 属于 §2.4 三方兼容协议扩展，是草案（见文首说明）。mock 与 fixture 证据不能替代真实三方服务的 smoke；真实调用证据按协议和服务分别记录。下文错误码用名称表示，编号见 §8.1。

#### AC15 Anthropic Provider 创建与模型分页

Given 用户选择 `anthropic` 厂商，或选择 `custom` 厂商的 Anthropic 兼容端点，填写有效 base_url 和 API Key

When 测试连接并保存，上游模型列表分两页返回

Then 请求带该画像规定的鉴权头和 `anthropic-version`（通用画像同时带 `Authorization: Bearer` 与 `x-api-key`）；按 `after_id` 取完两页，全部缓存为 `source=remote`；Provider DTO 不含 secret

#### AC16 厂商选择、画像匹配与高级选项校验

Given 厂商列表有 OpenAI、Anthropic 和自定义三项；三个 `openai` 厂商的 Provider，base_url 主机分别是 `api.openai.com`、`api.openai.com.evil.example` 和另一个三方主机；一个 `custom` 厂商选 Anthropic 兼容端点的 Provider

When 通过 `list_provider_vendors` 读取厂商并保存、调用这些 Provider；另提交不存在的 `vendor_id`、厂商没有对应端点的 `provider_type`（例如 `anthropic` 厂商配 `openai_compatible`），以及非法的 `auth_style` 或 `allow_insecure_http` 取值；再读取两条没有 `vendor_id` 的旧 Provider，一条主机是 `api.openai.com`，一条是三方主机

Then 厂商列表返回各项的 `vendor_id`、显示名和端点（适配器、端点名、默认 base_url，`custom` 的默认值为空）；第一个使用 OpenAI 官方画像，后两个和 `custom` 的 Provider 使用通用画像，画像不改变请求的目标主机；不存在的 `vendor_id` 或没有对应端点的组合被拒绝（`unsupported_provider_type`），非法的高级选项被拒绝（`request_not_supported`），都不产生 Provider 和凭据引用；旧 Provider 按主机推断：官方主机的归 `openai`，三方主机的归 `custom`，调用行为不变

#### AC17 无模型列表接口的服务与手动模型

Given 上游的模型列表接口返回 404

When 用户填写模型名并测试连接；之后手动添加两个模型，其中一个重复添加，并为其中一个指定 `kind`

Then 提示“该服务没有模型列表接口”，只发一次 `max_tokens=16` 的探测请求，任何 2xx 都算成功；手动模型保存为 `source=manual`，重复添加不产生第二行，指定的 `kind` 被采用；刷新模型列表只替换 `source=remote` 的行，手动模型保留

#### AC18 Anthropic 同步调用归一

Given `anthropic` Provider；请求以一条 system 消息开头，后接两条连续 user 消息，未给 `max_tokens`

When 同步调用

Then 上游请求的 system 在顶层，两条 user 合并为一条多内容块消息，`max_tokens` 取模型的 `max_output_tokens`，没有则取画像默认值；响应 `finish_reason` 为 `stop`，`usage.total_tokens` 等于输入加输出，写入成功的使用记录

#### AC19 Anthropic 流式归一

Given `anthropic` Provider；上游流式事件被拆在多个网络包中

When 流式调用

Then 文本增量按顺序输出，不丢字；最后一块带 `usage` 与 `finish_reason`；流结束时写入一条成功的使用记录

#### AC20 流中途错误

Given 两种协议的流式调用已输出部分内容

When 上游发出错误事件或断开连接

Then 调用方收到一次 `stream_interrupted`（1001007），已收到的内容由调用方保留；宿主不重试、不重放；使用记录 `success=false`

#### AC21 OpenAI 兼容 SSE 解码

Given `openai_compatible` 流式调用，请求带 `stream_options.include_usage`

When 上游一个网络包含多个事件、一个事件跨两个网络包，并夹有注释行和 `choices` 为空的用量块

Then 增量按顺序完整输出，不产生空块；用量写入使用记录；无法解析的事件按 `stream_interrupted` 结束，不静默丢弃

#### AC22 Anthropic 工具调用归一

Given 工作台任务带工具，Provider 为 `anthropic`

When 模型返回 `tool_use`，用户确认执行

Then `ToolCall.function.arguments` 是与 `openai_compatible` 相同的 JSON 字符串；下一轮请求中 assistant 的 `tool_use` 紧跟 user 的 `tool_result`；D022 的提案、逐次确认和阻断行为在两种协议上一致

#### AC23 推理配置与推理文本

Given 调用方设置推理档位

When 分别经 `openai_compatible` 通用画像、`anthropic` 通用画像、`anthropic` 官方画像，以及一个上游把推理文本放在 `reasoning_content` 或 `reasoning` 字段的服务调用

Then 推理配置按画像的推理参数模板映射为上游参数；推理文本进入 `reasoning`（流式为 `reasoning_delta`），不混入 `content`；档位在该画像下没有映射时返回 `request_not_supported`，上游 0 次请求

#### AC24 推理内容在工具循环中回传

Given 工作台工具循环；一种情形是 `anthropic` Provider 在 `tool_use` 前返回 thinking 块（包括调用方未要求推理、模型默认思考的情形）；另一种情形是 `openai_compatible` 上游在上一轮响应的 assistant 消息里带了推理字段（`reasoning_content`、`reasoning` 或 `reasoning_details`）

When 用户确认工具后宿主发出下一轮请求

Then Anthropic 上一轮 assistant 的 thinking 和 redacted_thinking 块（含签名）原样位于该消息开头；`openai_compatible` 按上一轮实际出现的字段名把推理内容随 assistant 消息原样回传，上游没有返回推理字段时不添加；各轮的 system 和工具定义不变；推理内容不出现在 SQLite、使用记录、审计和日志中

#### AC25 结构化输出

Given 调用方在非流式调用中给出 `response_format`（JSON Schema）

When 分别经两种协议调用；其中一次上游返回不符合 schema 的内容

Then 符合时 `structured` 为解析后的对象；不符合（含返回空内容或非 JSON）时返回 `structured_output_invalid`，宿主不自动重试；画像声明的结构化输出方式不被上游接受时，返回上游错误的归一结果（`provider_request_rejected`），不在本地预判

#### AC26 不允许的参数组合在本地拒绝

Given 一个 `anthropic` Provider 和一个 `openai_compatible` Provider

When 请求在协议上不可能成立（`anthropic` 的 system 消息不在开头，或消息以 assistant 结尾，或首条不是 user），或违反所用画像明确声明的约束（官方 Anthropic 画像下设置 `temperature`），或流式调用带 `response_format`

Then 宿主返回 `request_not_supported`，mock 上游收到 0 次请求；同样的请求发给没有内置画像的主机时，只有协议上不可能成立的情形被本地拒绝，其余原样发送，由上游决定

#### AC27 模型自动分类

Given 一个 `openai_compatible` 上游的模型列表包含 chat、embedding、rerank、图片生成、视频生成、语音合成、语音转写、审核各类的模型 ID，以及一个无法匹配任何规则的 ID；另有一个 `anthropic` 上游

When 拉取并保存模型列表，再读取

Then 每类模型的 `kind` 与规则表一致（技术设计“模型分类规则”的全部用例），无法匹配的归为 `text-generation` 且 `kind_source=default`；`anthropic` 的模型全部是 `text-generation` 且 `kind_source=protocol`；读取得到的 `model_type` 等于保存的 `kind`；`supports_streaming` 对 `text-generation` 为真；`max_output_tokens` 取自上游文档化字段，没有则为空

#### AC28 宽容读取

Given 一个 `openai_compatible` 上游响应缺少 `usage`、`choices` 为空、带有未知字段和未知的 `finish_reason`；模型列表含缺少 `id`、重复 `id` 和超长 `id` 的项

When 同步调用并拉取模型列表

Then 调用成功，`usage` 记 0，`content` 为空串，未知结束原因按 `stop` 处理；模型列表只保留合法且不重复的项；这些情形都不触发重试

#### AC29 错误归一

Given 两种协议的上游分别返回 401、402、403、404（模型不存在）、400、上下文超长、413、429、额度耗尽的 429、5xx、504、529

When 调用且未配置重试

Then 依次得到 `invalid_api_key`、`provider_quota_exhausted`、`provider_permission_denied`、`model_not_found`、`provider_request_rejected`、`context_length_exceeded`、`provider_request_rejected`、`rate_limit_exceeded`、`provider_quota_exhausted`、`upstream_server_error`、`timeout`、`upstream_server_error`；错误消息不包含上游错误正文和密钥

#### AC30 重试分类

Given Provider 配置 `retry_count=3`

When 上游分别返回带 `retry-after` 的 429、503、400；以及流式调用在已输出内容后断开

Then 429 与 503 按退避重试，429 的等待取 `retry-after` 与 `max_delay` 中较小的值；400 和额度耗尽的 429 不重试，分别返回 `provider_request_rejected` 和 `provider_quota_exhausted`；流式调用开始输出后不重试；重试至少发生一次后仍失败才返回 `max_retries_exceeded`（1001008）

#### AC31 超时分层

Given 一个流式调用持续超过 60 秒，事件间隔小于流式空闲超时；另一个流式调用在空闲超时内没有收到任何字节

When 两个调用分别进行

Then 第一个完整结束，不被截断；第二个返回 `timeout`（1001006），保留已收到的内容，不重试

#### AC32 HTTPS 策略

Given base_url 为 `http://` 或其他非 `http`/`https` 的 scheme（例如 `file://`）

When 保存配置；或调用一个此前已保存的同类配置

Then `file://` 等非 http/https 地址一律被拒绝；回环地址和私网字面 IP（`localhost`、`127.0.0.1`、`[::1]`、`10.x`、`172.16–31.x`、`192.168.x`）的 `http://` 可以保存和调用，并带明文传输提示；其他主机的 `http://` 在未设置 `allow_insecure_http` 时被拒绝（`insecure_endpoint`），已保存配置的调用在发送前被拒绝，上游 0 次请求；设置后可以保存，界面显示风险说明

#### AC33 流式 IPC 与取消

Given 前端通过 `call_provider_stream` 发起流式调用，并收到 `stream_id`

When 调用 `cancel_provider_stream`；或发起窗口关闭

Then 宿主停止读取上游，只发出一次中断事件；对已结束或不属于该窗口的 `stream_id`，`cancel_provider_stream` 返回 `false`；写入一条 `success=false`、`error_code` 为 1001007 的使用记录

#### AC34 不跟随重定向

Given 上游对模型列表、同步调用和流式调用分别返回 3xx，`Location` 指向另一个主机

When 调用

Then 宿主不向 `Location` 发出任何请求，凭据头只发往 base_url 的主机；三种调用都返回 `provider_response_invalid`，不重试

#### AC35 保存时由宿主重算模型数据

Given 前端提交的模型列表中，某项带有伪造的 `kind`、能力、主键和上下文长度，另一项的 `model_id` 非法（空串或超过 255 字符）

When 调用 `create_provider_with_models`

Then 只采用 `model_id` 与 `model_name`，`kind`、能力、主键由宿主重新计算，上下文长度和最大输出长度不采用前端的值；非法项被拒绝，整个创建不产生 Provider 和凭据引用

#### AC36 参数降级重试

Given 一个上游以 400 拒绝 `stream_options`，另一个以 400 拒绝 `tool_choice`，第三个以 400 拒绝一个不在白名单内的参数

When 分别发起流式调用和带工具的调用

Then 前两个去掉被点名的参数后重试一次并成功，每次调用最多降级一次，降级不持久化、不写入上游错误正文；第三个直接返回 `provider_request_rejected`，不重试

#### AC37 连接测试诊断

Given 上游分别返回 401、403、200 的 HTML 页面、404、连接被拒绝

When 点击“测试连接”

Then 依次得到 `invalid_api_key`、`provider_permission_denied`、`provider_response_invalid`、转探测流程、网络错误；创建 Provider 时 401 返回 `invalid_api_key` 而不是 `provider_unavailable`；界面提示符合 §2.4.5 的诊断表

#### AC 逐项测试设计

证据状态按 spec-docs-pro 词表记录：未创建 / 已创建 / 静态核验 / 本地通过 / 预发通过 / 生产通过 / 失败 / 不适用。原“初始资产状态”列的内容移入“备注”，不代表升格。

| AC | 验收重点 | 测试层级 | 目标资产 | 目标命令 | 证据状态 | 备注 |
| --- | --- | --- | --- | --- | --- | --- |
| AC01 | Provider 正常创建 | 后端 API + 数据库 | `crates/dgos-db/src/provider_scope_tests.rs::provider_and_initial_models_commit_together` | `cargo test --offline -p dgos-db provider_and_initial_models_commit_together` | 已创建 | 已有自动化资产（覆盖原子保存，不覆盖真实 Provider 自动拉取） |
| AC02 | API Key 无效检测 | 后端 API | `crates/dgos-providers/tests/provider_connection_tests.rs` | `cargo test --offline -p dgos-providers invalid_api_key` | 未创建 | 无对应自动化测试 |
| AC03 | 模型列表获取与缓存 | 后端 API + 数据库 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP） |
| AC04 | 模型分类标签正确 | 后端逻辑 | `crates/dgos-providers/src/adapters/openai.rs::test_classify_model` | `cargo test --offline -p dgos-providers test_classify_model` | 已创建 | 已有自动化资产 |
| AC05 | 同步调用成功 | 后端 API + 外部集成 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP，非真实 Provider） |
| AC06 | 流式调用成功 | 后端 API + 外部集成 | `crates/dgos-providers/tests/stream_call_tests.rs` | `cargo test --offline -p dgos-providers stream_call` | 未创建 | 无对应自动化测试 |
| AC07 | 流式输出中断处理 | 后端 API | `crates/dgos-providers/src/manager.rs::cancellable_stream_emits_one_interruption_without_replay` | `cargo test --offline -p dgos-providers cancellable_stream_emits_one_interruption_without_replay` | 已创建 | 已有自动化资产（仅覆盖取消产生一次中断，不覆盖已接收内容的保存） |
| AC08 | 并发调用支持 | 后端 API + 并发测试 | `crates/dgos-providers/tests/concurrent_call_tests.rs` | `cargo test --offline -p dgos-providers concurrent_calls` | 未创建 | 无对应自动化测试 |
| AC09 | 可配置重试策略 | 后端逻辑 | `crates/dgos-providers/src/retry.rs::test_retry_success_after_failures` | `cargo test --offline -p dgos-providers test_retry_success_after_failures` | 已创建 | 已有自动化资产（覆盖重试，不覆盖指定 1s/2s/4s 间隔） |
| AC10 | 重试失败后返回错误 | 后端逻辑 | `crates/dgos-providers/src/retry.rs::test_retry_exhausted` | `cargo test --offline -p dgos-providers test_retry_exhausted` | 已创建 | 已有自动化资产 |
| AC11 | 多 Provider 排序 | 后端 API + 前端集成 | `crates/dgos-db/src/provider_scope_tests.rs::scoped_provider_crud_and_transactional_order_preserve_normal_lifecycle` | `cargo test --offline -p dgos-db scoped_provider_crud_and_transactional_order_preserve_normal_lifecycle` | 已创建 | 已有自动化资产（覆盖事务排序，不覆盖拖拽 UI） |
| AC12 | Provider 启用/禁用 | 后端 API | `tests/providers.test.tsx::uses the dedicated toggle command and updates both provider collections` | `npm test -- --run tests/providers.test.tsx` | 已创建 | 已有自动化资产（Vitest IPC mock） |
| AC13 | 手动刷新模型列表 | 后端 API + 数据库 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP） |
| AC14 | 使用记录保存 | 后端 API + 数据库 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP，检查记录查询及调用 provenance） |
| AC15 | Anthropic 创建与模型分页 | 后端 API + 数据库 | `crates/dgos-providers/tests/anthropic_adapter.rs` | `cargo test --offline -p dgos-providers --test anthropic_adapter` | 未创建 | 草案；mockito fixture；官方画像与通用画像各一例 |
| AC16 | 厂商选择、画像匹配与高级选项校验 | 后端逻辑 + 前端集成 | `crates/dgos-providers/src/vendor.rs`；`tests/provider-vendors.test.tsx` | `cargo test --offline -p dgos-providers vendor`；`npm test -- --run tests/provider-vendors.test.tsx` | 未创建 | 草案；含域名标签边界的反例、旧数据按主机推断（官方主机与三方主机各一例）、多端点选择、前端从命令读取厂商而不自带列表 |
| AC17 | 无模型列表接口与手动模型 | 后端 API + 数据库 | `crates/dgos-db/src/provider_scope_tests.rs`；`crates/dgos-providers/tests/connection_probe.rs` | `cargo test --offline -p dgos-db manual_models`；`cargo test --offline -p dgos-providers --test connection_probe` | 未创建 | 草案；需要 `models.source`、`models.kind` migration |
| AC18 | Anthropic 同步归一 | 后端 API + 外部集成 | `crates/dgos-providers/tests/anthropic_adapter.rs` | `cargo test --offline -p dgos-providers --test anthropic_adapter sync` | 未创建 | 草案；请求与响应 golden fixture |
| AC19 | Anthropic 流式归一 | 后端 API + 外部集成 | `crates/dgos-providers/tests/anthropic_stream.rs` | `cargo test --offline -p dgos-providers --test anthropic_stream` | 未创建 | 草案；拆包由解码器单元测试覆盖 |
| AC20 | 流中途错误 | 后端 API | `crates/dgos-providers/tests/stream_call_tests.rs` | `cargo test --offline -p dgos-providers --test stream_call_tests mid_stream_error` | 未创建 | 草案；两种协议各一例 |
| AC21 | OpenAI 兼容 SSE 解码 | 后端逻辑 | `crates/dgos-providers/src/sse.rs` | `cargo test --offline -p dgos-providers sse` | 未创建 | 草案；任意字节边界切分的单元测试 |
| AC22 | Anthropic 工具调用归一 | 后端 API + 跨功能 | `crates/dgos-providers/tests/anthropic_adapter.rs`；`crates/dgos-workbench/tests/tool_flow.rs` | `cargo test --offline -p dgos-providers --test anthropic_adapter tools`；`cargo test --offline -p dgos-workbench --test tool_flow anthropic` | 未创建 | 草案；`tool_flow.rs` 已存在，Anthropic 用例未创建 |
| AC23 | 推理配置与推理文本 | 后端 API | `crates/dgos-providers/tests/reasoning.rs` | `cargo test --offline -p dgos-providers --test reasoning` | 未创建 | 草案；三种画像与两种返回字段各一例 |
| AC24 | 推理内容回传 | 后端 API + 跨功能 | `crates/dgos-workbench/tests/tool_flow.rs` | `cargo test --offline -p dgos-workbench --test tool_flow reasoning_round_trip` | 未创建 | 草案；Anthropic 思考块与 `reasoning_content` 各一例；断言 SQLite 与日志中没有推理内容 |
| AC25 | 结构化输出 | 后端 API | `crates/dgos-providers/tests/structured_output.rs` | `cargo test --offline -p dgos-providers --test structured_output` | 未创建 | 草案 |
| AC26 | 请求本地拒绝的范围 | 后端逻辑 | `crates/dgos-providers/tests/anthropic_adapter.rs`；`crates/dgos-providers/tests/openai_adapter.rs` | `cargo test --offline -p dgos-providers --test anthropic_adapter rejects_`；`cargo test --offline -p dgos-providers --test openai_adapter passthrough_` | 未创建 | 草案；断言 mock 命中次数：拒绝为 0，透传为 1 |
| AC27 | 模型自动分类 | 后端逻辑 + 数据库 | `crates/dgos-providers/src/classify.rs`；`crates/dgos-db/src/provider_scope_tests.rs` | `cargo test --offline -p dgos-providers classify`；`cargo test --offline -p dgos-db model_kind_roundtrip` | 未创建 | 草案；规则用例表见技术设计；现有 `test_classify_model` 属于 AC04，不计入 |
| AC28 | 宽容读取 | 后端 API | `crates/dgos-providers/tests/tolerant_reader.rs` | `cargo test --offline -p dgos-providers --test tolerant_reader` | 未创建 | 草案；两种协议各一组畸形响应 |
| AC29 | 错误归一 | 后端逻辑 | `crates/dgos-providers/tests/error_mapping.rs` | `cargo test --offline -p dgos-providers --test error_mapping` | 未创建 | 草案；两种协议各 12 种情形 |
| AC30 | 重试分类 | 后端逻辑 | `crates/dgos-providers/src/retry.rs` | `cargo test --offline -p dgos-providers retry_classification` | 未创建 | 草案；现有 `retry.rs` 用例不覆盖本 AC |
| AC31 | 超时分层 | 后端 API | `crates/dgos-providers/tests/timeouts.rs` | `cargo test --offline -p dgos-providers --test timeouts` | 未创建 | 草案；用本地 TCP 替身控制发送节奏，不真实等待 60 秒 |
| AC32 | 端点策略（scheme 与明文 HTTP） | 后端逻辑 | `crates/dgos-providers/src/endpoint.rs` | `cargo test --offline -p dgos-providers endpoint_policy` | 未创建 | 草案；包含已保存的旧配置和 `file://` |
| AC33 | 流式 IPC 与取消 | 宿主命令 + 契约检查 | `src-tauri/src/commands/provider_commands.rs`；`scripts/check-host-contracts.mjs` | `cargo test --offline -p dgos provider_stream`；`npm run contracts:check` | 未创建 | 草案；真实 WebView 中的取消另行记录 |
| AC34 | 不跟随重定向 | 后端 API | `crates/dgos-providers/tests/redirects.rs` | `cargo test --offline -p dgos-providers --test redirects` | 未创建 | 草案；第二个 mock 主机断言 0 次命中 |
| AC35 | 保存时由宿主重算模型数据 | 后端 API + 数据库 | `crates/dgos-db/src/provider_scope_tests.rs`；`crates/dgos-providers/src/credential_tests.rs` | `cargo test --offline -p dgos-db renderer_models_recomputed`；`cargo test --offline -p dgos-providers renderer_models_recomputed` | 未创建 | 草案 |
| AC36 | 参数降级重试 | 后端 API | `crates/dgos-providers/tests/param_downgrade.rs` | `cargo test --offline -p dgos-providers --test param_downgrade` | 未创建 | 草案；断言每次调用最多一次降级、不持久化 |
| AC37 | 连接测试诊断 | 后端 API + 前端集成 | `crates/dgos-providers/tests/provider_connection_tests.rs`；`tests/providers.test.tsx` | `cargo test --offline -p dgos-providers --test provider_connection_tests`；`npm test -- --run tests/providers.test.tsx` | 未创建 | 草案；与 AC02 共用文件，AC02 本身仍为“未创建” |


### 5.2 边界条件测试

| 边界条件 | 测试场景 | 预期结果 |
| --- | --- | --- |
| 空 Provider 列表 | 用户未配置任何 Provider | 提示"请先配置 Provider" |
| 重复 Provider 名称 | 创建同名 Provider | 允许（名称可重复，ID 不重复） |
| API Key 包含空白 | 粘贴 key 时前后有空格、换行 | 自动 trim 前后空白；内部空白或非 ASCII 不作为 Bearer key 保存 |
| 超长 Prompt | Prompt 长度超过模型限制 | 返回错误"超过最大长度" |
| 并发超过 100 | 100+ 并发请求 | 正常处理，不崩溃 |
| 网络完全断开 | 调用时无网络 | 返回"网络错误"，不无限等待 |
| Anthropic 请求未给 max_tokens | 模型有 `max_output_tokens` / 没有 | 取该值 / 取画像默认值；按预算控制思考时要求预算小于 max_tokens，否则 `request_not_supported`（三方兼容协议扩展，草案） |
| 手动模型不在目录中 | 用户添加任意模型 ID | 能力全部为“未知”，请求照常发出（多协议扩展，草案） |
| 兼容服务不支持 `stream_options` | 上游以 400 点名该参数 | 去掉后重试一次（§2.4.6 第 11 项）；上游没有返回用量时记 0 tokens（三方兼容协议扩展，草案） |
| 模型列表很大或含畸形项 | 数千项、缺 `id`、重复、超长 | 按 §2.4.7 宽容读取规则过滤；超过上限截断（三方兼容协议扩展，草案） |
| 对非对话模型发起对话调用 | 用户或应用层选了 `embedding` 一类的模型 | 宿主不拦截，上游错误按 §8 归一返回；是否限定留给场景绑定设计（三方兼容协议扩展，草案） |
| Anthropic base_url 以 `/v1` 结尾 | 用户粘贴 `https://api.anthropic.com/v1` | 按 §2.4.6 第 2 项处理（多协议扩展，草案） |
| 上游返回空 `choices` 或空内容块 | 非流式调用 | `content` 为空串，`finish_reason` 按上游值映射；不当作成功的结构化输出（多协议扩展，草案） |

## 6. 接口设计

本节命令签名为早期草案；当前已注册命令见 `src-tauri/src/lib.rs`、`src-tauri/src/commands/provider_commands.rs` 和 [接口契约 §4.0](../../../04-技术架构/当前版本/V1-接口契约.md#40-当前桌面实施契约d023)。

### 6.1 Tauri Commands（桌面端）

```rust
// Provider CRUD
#[tauri::command]
async fn list_providers(user_id: String) -> Result<Vec<Provider>>;

#[tauri::command]
async fn create_provider(config: ProviderConfig) -> Result<Provider>;

#[tauri::command]
async fn update_provider(id: String, config: ProviderConfig) -> Result<Provider>;

#[tauri::command]
async fn delete_provider(id: String) -> Result<()>;

#[tauri::command]
async fn test_provider_connection(config: ProviderConfig) -> Result<bool>;

// 模型管理
#[tauri::command]
async fn list_models(provider_id: String) -> Result<Vec<Model>>;

#[tauri::command]
async fn refresh_models(provider_id: String) -> Result<Vec<Model>>;

// Provider 调用
#[tauri::command]
async fn call_provider(
    provider_id: String,
    model_id: String,
    request: ChatRequest
) -> Result<ChatResponse>;

#[tauri::command]
async fn stream_provider(
    provider_id: String,
    model_id: String,
    request: ChatRequest
) -> Result<Stream<ChatChunk>>;

// 排序管理
#[tauri::command]
async fn update_provider_order(provider_ids: Vec<String>) -> Result<()>;

// 使用记录
#[tauri::command]
async fn list_usage_logs(
    user_id: String,
    limit: Option<usize>
) -> Result<Vec<UsageLog>>;
```

### 6.2 HTTP API（局域网）

```
POST /api/v1/providers                    # 创建 Provider
GET  /api/v1/providers                    # 获取 Provider 列表
PUT  /api/v1/providers/:id                # 更新 Provider
DELETE /api/v1/providers/:id              # 删除 Provider
POST /api/v1/providers/:id/test           # 测试连接

GET  /api/v1/providers/:id/models         # 获取模型列表
POST /api/v1/providers/:id/models/refresh # 刷新模型列表

POST /api/v1/providers/:id/call           # 调用 Provider（同步）
POST /api/v1/providers/:id/stream         # 调用 Provider（流式）

PUT  /api/v1/providers/order              # 更新排序

GET  /api/v1/usage-logs                   # 使用记录
```

详见 [V1 接口契约](../../../04-技术架构/当前版本/V1-接口契约.md)。

### 6.3 三方兼容协议扩展的命令与字段（草案）

本节在 D-待编号-B 冻结后登记到接口契约 §4.0；登记前不是已注册命令。新增字段全部可选，旧请求和旧响应 JSON 不变。

```typescript
interface ChatRequest {            // 现有字段：messages、temperature、max_tokens、stream
  reasoning?: {                    // 档位原样交给上游，宿主不在本地校验子集（§2.4.6 第 6 项）
    effort?: 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max';
    budget_tokens?: number;        // 只用于画像按预算控制思考的服务
  };
  response_format?: { type: 'json_schema'; name: string; schema: object; strict?: boolean };
}
interface ChatResponse {           // 现有字段：content、usage、audit_warning
  reasoning?: string;              // 可展示的推理文本
  finish_reason?: 'stop' | 'length' | 'tool_calls' | 'content_filter';
  structured?: unknown;            // 通过 schema 校验后的对象
}
type ProviderStreamEvent =
  | { kind: 'delta'; text: string }
  | { kind: 'reasoning_delta'; text: string }
  | { kind: 'done'; finish_reason: string; usage: TokenUsage }
  | { kind: 'error'; code: number; message: string };
interface ProviderVendorEndpoint {
  provider_type: string;           // 适配器：openai_compatible | anthropic
  display_name: string;            // 端点名称，如“OpenAI 兼容”
  default_base_url: string | null; // “自定义”的端点为 null，用户必须填写
}
interface ProviderVendor {         // 宿主内置厂商表的公开字段；首批 openai、anthropic、custom
  vendor_id: string;               // 用字符串而不是联合类型，新增厂商不改契约
  display_name: string;
  endpoints: ProviderVendorEndpoint[]; // 至少一项；只有一项时选厂商即确定适配器
}
type ModelKind =
  | 'text-generation' | 'embedding' | 'rerank' | 'image-generation' | 'video-generation'
  | 'speech-synthesis' | 'speech-transcription' | 'moderation';
type Hint = 'supported' | 'unsupported' | 'unknown';
interface ModelCapabilities {      // 能力提示：只供展示和后续场景设计，宿主不据此拦截
  streaming: Hint; vision: Hint; tools: Hint; reasoning: Hint; structured_output: Hint;
}
// Model：现有字段保留；model_type 承载 kind（旧值 'chat' 读取时视为 'text-generation'）；
// 追加 kind_source: 'protocol' | 'rule' | 'default' | 'manual'、source: 'remote' | 'manual'、
// capabilities: ModelCapabilities；max_output_tokens 由上游文档化字段填充，没有则为空。
// ProviderConfig.config 追加 vendor_id: string、auth_style: 'bearer' | 'x-api-key' | 'both'、allow_insecure_http: boolean。
// ToolMessage.reasoning（含 opaque 思考块）只在 Rust 内部使用，不序列化给 renderer。
```

| Command / IPC 参数 | `CommandResponse` 的 data | 说明 |
| --- | --- | --- |
| `list_provider_vendors({})` | `ProviderVendor[]` | 只读，返回宿主内置厂商表的公开字段；前端不自带厂商列表 |
| `list_models_for_config({config})` | `Model[]` | 现有命令：预览返回的模型带 `kind` 和能力提示，不写库 |
| `create_provider_with_models({config, models})` | `DesktopProvider` | 现有命令：`models` 中只采用 `model_id` 与 `model_name`，其余由宿主重新计算（AC35） |
| `add_manual_model({providerId, modelId, kind?})` | `Model` | 幂等；可选 `kind`，默认按规则；递增 `config_revision` |
| `remove_manual_model({providerId, modelId})` | `null` | 只删除 `source=manual` 的行；不存在时也返回成功 |
| `call_provider_stream({providerId, modelId, request, onEvent})` | `{stream_id}` | 事件只推送到调用方传入的 Tauri Channel，不使用全局事件（同一 realm 的其他页面可以监听全局事件，D042）；名称沿用接口契约 §5.2 |
| `cancel_provider_stream({streamId})` | `boolean` | 只取消本窗口发起的流；对已结束或不属于本窗口的流返回 `false` |
| `test_provider_connection({config, probeModel?})` | `boolean` | 新增可选 `probeModel`，用于没有模型列表端点的服务（§2.4.5） |

用户身份由宿主派生，入参不含 `user_id`（D023）。模块 `providers.call`（D051）的 `request` 与本节 `ChatRequest` 同源，自动获得新的可选字段，命令入参白名单不变。

## 7. 技术设计

### 7.1 核心 Trait 定义

```rust
// crates/dgos-providers/src/traits.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// 获取 Provider 类型标识
    fn provider_type(&self) -> &str;
    
    /// 获取模型列表
    async fn list_models(&self) -> Result<Vec<Model>>;
    
    /// 同步调用
    async fn call(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<ChatResponse>;
    
    /// 流式调用
    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<Stream<ChatChunk>>;
    
    /// 测试连接
    async fn test_connection(&self) -> Result<bool>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<Capability>,
    pub context_length: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Capability {
    TextGeneration,
    ImageGeneration,
    VideoGeneration,
    AudioGeneration,
}
```

### 7.2 OpenAI 适配器实现

```rust
// crates/dgos-providers/src/adapters/openai.rs

pub struct OpenAIAdapter {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    retry_config: Option<RetryConfig>,
}

impl OpenAIAdapter {
    pub fn new(base_url: String, api_key: String, retry_config: Option<RetryConfig>) -> Self {
        Self {
            base_url,
            api_key,
            client: reqwest::Client::new(),
            retry_config,
        }
    }
    
    // B01-26: 给模型打分类标签（内置维护）
    fn classify_model(&self, model_id: &str) -> Vec<Capability> {
        match model_id {
            "gpt-4" | "gpt-3.5-turbo" | "gpt-4-turbo" => {
                vec![Capability::TextGeneration]
            }
            "dall-e-3" | "dall-e-2" => {
                vec![Capability::ImageGeneration]
            }
            _ => vec![],
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAIAdapter {
    fn provider_type(&self) -> &str {
        "openai_compatible"
    }
    
    // B01-27: 获取模型列表
    async fn list_models(&self) -> Result<Vec<Model>> {
        let url = format!("{}/models", self.base_url);
        let response = self.client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        
        let models_data: OpenAIModelsResponse = response.json().await?;
        
        let models = models_data.data.into_iter()
            .map(|m| Model {
                id: m.id.clone(),
                name: m.id,
                capabilities: self.classify_model(&m.id), // B01-26
                context_length: None,
            })
            .collect();
        
        Ok(models)
    }
    
    // B01-28: 同步调用（支持重试）
    async fn call(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = OpenAIChatRequest {
            model: model_id.to_string(),
            messages: request.messages,
            temperature: request.temperature,
            stream: false,
        };
        
        // B01-29: 重试逻辑
        let mut attempts = 0;
        let max_attempts = self.retry_config
            .as_ref()
            .map(|c| c.max_retries + 1)
            .unwrap_or(1);
        
        loop {
            attempts += 1;
            
            let response = self.client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;
            
            match response {
                Ok(resp) if resp.status().is_success() => {
                    let data: OpenAIChatResponse = resp.json().await?;
                    return Ok(ChatResponse {
                        content: data.choices[0].message.content.clone(),
                        tokens: data.usage.total_tokens,
                    });
                }
                Ok(resp) if resp.status() == 500 && attempts < max_attempts => {
                    // B01-30: 500 错误重试，指数退避
                    let delay = 2u64.pow(attempts - 1);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                Ok(resp) => {
                    // B01-31: 其他错误不重试
                    return Err(Error::ProviderError(resp.status()));
                }
                Err(e) if attempts < max_attempts => {
                    // B01-32: 网络错误重试
                    let delay = 2u64.pow(attempts - 1);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                Err(e) => {
                    return Err(Error::NetworkError(e));
                }
            }
        }
    }
    
    // B01-33: 流式调用
    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<Stream<ChatChunk>> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = OpenAIChatRequest {
            model: model_id.to_string(),
            messages: request.messages,
            temperature: request.temperature,
            stream: true,
        };
        
        let response = self.client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        
        // B01-34: 转换为 SSE Stream
        let stream = response
            .bytes_stream()
            .map(|chunk| {
                // 解析 SSE 格式
                parse_sse_chunk(chunk)
            });
        
        Ok(stream)
    }
    
    // B01-35: 测试连接
    async fn test_connection(&self) -> Result<bool> {
        let url = format!("{}/models", self.base_url);
        let response = self.client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        
        Ok(response.status().is_success())
    }
}
```

### 7.3 分支到测试追踪表

本节是适配器层分支，编号 B01-26～B01-35；管理器层的同步与流式分支见 §10.2。编号规则与改号对照见 [§10.2 分支编号说明](#分支编号说明2026-10-29)。

| 分支 ID | 分支描述 | AC 映射 | E2E 映射 | NFR 映射 |
| --- | --- | --- | --- | --- |
| B01-26 | 模型分类标签 | AC04 | - | - |
| B01-27 | 获取模型列表 | AC03 | - | - |
| B01-28 | 同步调用 | AC05 | V1-E2E-01 | - |
| B01-29～32 | 重试逻辑 | AC09, AC10 | - | V1-NFR-001（可用性） |
| B01-30 | 500 错误重试 | AC09 | - | - |
| B01-31 | 其他错误不重试 | AC10 | - | - |
| B01-32 | 网络错误重试 | AC09 | - | - |
| B01-33 | 流式调用 | AC06 | V1-E2E-01 | - |
| B01-34 | SSE Stream 解析 | AC06, AC07 | - | - |
| B01-35 | 测试连接 | AC02 | - | - |

### 7.4 多协议扩展技术设计

三方兼容协议扩展（AC15～AC37）的伪代码、协议映射、厂商表与兼容画像、模型分类规则和分支 B01-36～B01-71 见[技术设计](01-Provider适配框架-技术设计.md)。§7.1、§7.2 是早期草图，不作为当前代码或扩展设计的依据。

## 8. 错误码设计

| 错误码 | 错误信息 | 场景 | 用户操作建议 |
| --- | --- | --- | --- |
| 1001001 | provider_not_found | Provider 不存在 | 检查 Provider ID |
| 1001002 | invalid_api_key | API Key 无效 | 检查 API Key 是否正确 |
| 1001003 | provider_unavailable | Provider 不可用 | 检查网络或 Provider 状态 |
| 1001004 | model_not_found | 模型不存在 | 刷新模型列表或检查模型 ID |
| 1001005 | rate_limit_exceeded | 超过速率限制 | 稍后重试或切换 Provider |
| 1001006 | timeout | 请求超时 | 检查网络或重试 |
| 1001007 | stream_interrupted | 流式输出中断 | 重新生成 |
| 1001008 | max_retries_exceeded | 重试次数耗尽 | 检查 Provider 状态或配置 |

本表错误码编号与 `crates/dgos-types/src/error.rs::error_code()` 一致。错误码文档中曾登记的另一套含义已按 [D038](../../../06-决策记录/V1-冻结决策.md#d038-错误码以代码已发出的映射为准) 作废。1001003 也用于 Provider 已禁用和连接测试失败；另有 1001012（配置或模型刷新已变化）、1001015/1001016（凭据，D023）和通用码 1000000，完整列表见 [V1 错误码 §3.1](../../../04-技术架构/当前版本/V1-错误码.md#31-provider-适配框架01)。

### 8.1 多协议扩展拟新增错误码（草案）

以下编号是拟分配值，尚未登记到 [V1 错误码](../../../04-技术架构/当前版本/V1-错误码.md)。按错误码文档“先分配编号，再编码”的原则，登记前代码不得发出这些编号；AC 用名称引用，登记时编号若有变化不影响 AC 原文。已作废的 1001010、1001011、1001013 不复用。

| 拟分配 | 错误信息 | 场景 | HTTP 状态码 |
| --- | --- | --- | --- |
| 1001014 | upstream_server_error | 已有预留码改为使用中：上游 5xx 或 529，未配置重试 | 502 |
| 1001017 | provider_request_rejected | 上游以 400、413 或 422 拒绝请求参数 | 400 |
| 1001018 | context_length_exceeded | 上游判定输入超出上下文窗口 | 400 |
| 1001019 | provider_permission_denied | 上游 403：Key 有效但无权限或地区受限 | 403 |
| 1001020 | request_not_supported | 协议上不可能成立的请求，或违反所用画像明确声明的约束，或高级选项取值非法，未发出上游请求 | 400 |
| 1001021 | structured_output_invalid | 上游输出为空、不是 JSON 或不符合调用方给出的 schema | 502 |
| 1001022 | unsupported_provider_type | 未知 `provider_type`，或 `vendor_id` 不存在、该厂商没有 `provider_type` 对应的端点（取代现在返回的 1000000） | 400 |
| 1001023 | insecure_endpoint | scheme 不是 http/https，或非回环、非私网主机使用 `http://` 且未设置 `allow_insecure_http` | 400 |
| 1001024 | provider_quota_exhausted | 上游 402，或 429 中的额度/花费上限耗尽（不可重试） | 402 |
| 1001025 | provider_response_invalid | 上游响应不是预期的 JSON、响应体超限，或收到 3xx（宿主不跟随重定向） | 502 |

错误语义变化（随 D-待编号-B 冻结）：未知协议从 1000000 改为 1001022；创建 Provider 时上游 401 返回 1001002，而不是 1001003（AC37）；配置了重试且至少重试过一次仍失败才返回 1001008，未配置重试时返回原始类别码；不可重试的错误保持原码，不再被改写为 1001008。

## 9. 非功能需求

### 9.1 性能需求

| 指标 | 目标 | 测量方式 |
| --- | --- | --- |
| 模型列表获取 | < 3 秒 | 集成测试 |
| 同步调用响应 | < 30 秒（取决于 Provider） | 集成测试 |
| 流式首字节延迟 | < 2 秒 | 集成测试 |
| 并发调用支持 | > 100 QPS | 压力测试 |
| 数据库查询 | < 50ms（Provider 列表） | 单元测试 |

### 9.2 安全需求

| 需求 | 实现方式 |
| --- | --- |
| API Key 安全存储 | D023：OS vault 保存 secret，SQLite 仅引用；旧活动明文字段显式迁移，失败保留供重试 |
| HTTPS 强制 | 调用 Provider 时强制 HTTPS；多协议扩展（草案）补充回环地址例外，保存和调用时都检查（AC32） |
| 敏感信息脱敏 | D023：不记录 API Key，包括前缀；日志只记录类别与资源引用；`x-api-key` 头与 Bearer 同等对待 |
| 推理内容不落盘 | 多协议扩展（草案）：推理文本和思考块不进 SQLite、使用记录、审计和日志；错误消息只含错误类别，不含上游错误正文 |

### 9.3 可用性需求

| 需求 | 实现方式 |
| --- | --- |
| 失败重试 | 可配置重试次数和间隔；多协议扩展（草案）限定可重试类别并遵守 `retry-after`（AC30） |
| 超时分层 | 多协议扩展（草案）：连接、非流式总期限、流式空闲三层，数值见 §2.4.6 第 3 项（AC31） |
| 错误提示 | 用户友好的错误信息 |
| 流式中断恢复 | 保存已接收部分，提示重新生成 |

## 10. 实现与验证

### 10.1 验证记录

**说明**：每次验证后更新本节，记录日期、命令、通过率和覆盖说明。格式遵循 spec-docs-pro 标准。
**状态与证据**：实现状态及测试通过证据仅维护在 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)，本节命令仅列验证目标，不代表已执行或通过。

#### 2026-10-26 后端单元测试

```bash
cargo test --offline -p dgos-providers --lib
```

**状态**：🔴 未开始

**目标覆盖**：Provider trait、模型分类、凭据边界及重试策略；具体自动化资产见 §5.1。

---

#### 2026-10-26 后端集成测试

```bash
cargo test --offline -p dgos-db provider_scope_tests
```

**状态**：🔴 未开始

**目标覆盖**：Provider/模型持久化与刷新；其余 AC 的现有资产及缺口见 §5.1。

---

#### 2026-10-26 前端集成测试

```bash
npm test -- --run tests/providers.test.tsx tests/provider-management.test.tsx
```

**状态**：🔴 未开始

**目标覆盖**：Provider 管理页面和 store 的现有 Vitest 覆盖；具体资产见 §5.1。

---

#### 2026-10-26 E2E 测试

**资产状态**：V1-E2E-01 为规划资产（未建立）；当前项目没有 E2E 测试框架，暂无可执行命令。

**状态**：🔴 未开始

**计划覆盖**：
- **V1-E2E-01: 完整配置 Provider 流程**
  1. 启动应用，登录
  2. 打开系统设置 → Provider 管理
  3. 点击"添加 Provider"
  4. 选择"OpenAI 兼容"
  5. 填写：名称="测试 OpenAI"、Base URL、API Key
  6. 点击"测试连接"→ 验证成功提示
  7. 保存配置
  8. 验证 Provider 出现在列表中
  9. 验证模型列表已自动获取
  10. 验证数据库记录正确保存

---

### 10.2 技术设计与分支追踪

#### 技术设计伪代码

**同步调用流程**：

```text
FUNCTION call_provider(provider_id, model_id, request) -> Result<Response>:
  // B01-01: 查询 Provider 配置
  provider = DB.query("SELECT * FROM providers WHERE id = ? AND enabled = TRUE", provider_id)
  IF NOT provider:
    RETURN Err(E_PROVIDER_NOT_FOUND)
  
  // B01-02: 构造适配器
  adapter = create_adapter(provider.provider_type, provider.base_url, provider.api_key)
  native_request = adapter.convert_request(model_id, request)
  
  // B01-03: 读取重试配置
  retry_config = parse_json(provider.config)
  max_retries = retry_config.retry_count OR 0
  
  // B01-04: 发起调用（带重试）
  attempt = 0
  WHILE attempt <= max_retries:
    TRY:
      start_time = now()
      response = HTTP.post(provider.base_url + "/chat/completions", native_request)
      latency_ms = now() - start_time
      
      // B01-05: 检查成功响应
      IF response.status == 200:
        result = adapter.convert_response(response.body)
        
        // B01-06: 记录使用日志
        INSERT INTO provider_usage_logs(
          provider_id, model_id, user_id,
          request_tokens, response_tokens, total_tokens,
          latency_ms, success = TRUE
        )
        
        RETURN Ok(result)
      
      // B01-07: 401 错误（API Key 无效）
      IF response.status == 401:
        RETURN Err(E_INVALID_API_KEY)  // 不重试
      
      // B01-08: 429 错误（速率限制）
      IF response.status == 429:
        IF attempt < max_retries:
          backoff_delay = calculate_backoff(attempt)  // 1s, 2s, 4s, 8s
          SLEEP(backoff_delay)
          attempt += 1
          CONTINUE
        ELSE:
          RETURN Err(E_RATE_LIMIT_EXCEEDED)
      
      // B01-09: 5xx 错误（服务器错误）
      IF response.status >= 500:
        IF attempt < max_retries:
          backoff_delay = calculate_backoff(attempt)
          SLEEP(backoff_delay)
          attempt += 1
          CONTINUE
        ELSE:
          RETURN Err(E_SERVER_ERROR)
      
      // B01-10: 其他错误
      RETURN Err(E_UNKNOWN_ERROR)
    
    CATCH (NetworkError as e):
      // B01-11: 网络错误
      IF attempt < max_retries:
        backoff_delay = calculate_backoff(attempt)
        SLEEP(backoff_delay)
        attempt += 1
        CONTINUE
      ELSE:
        RETURN Err(E_NETWORK_ERROR)
  
  RETURN Err(E_RETRY_EXHAUSTED)
```

**流式调用流程**：

```text
FUNCTION stream_provider(provider_id, model_id, request) -> Result<Stream>:
  // B01-18: 查询 Provider（复用 B01-01 逻辑）
  provider = DB.query("SELECT * FROM providers WHERE id = ? AND enabled = TRUE", provider_id)
  IF NOT provider:
    RETURN Err(E_PROVIDER_NOT_FOUND)
  
  // B01-19: 构造流式请求
  adapter = create_adapter(provider.provider_type, provider.base_url, provider.api_key)
  native_request = adapter.convert_request(model_id, request)
  native_request.stream = TRUE
  
// B01-20: 发起 SSE 流式调用
  TRY:
    sse_stream = HTTP.post_stream(provider.base_url + "/chat/completions", native_request)
    accumulated_text = ""
    
    // B01-21: 返回流处理器
    RETURN StreamHandler {
      on_chunk: FUNCTION(chunk):
        // B01-22: 解析 SSE 数据
        IF chunk.starts_with("data: "):
          data_str = chunk.trim_prefix("data: ")
          IF data_str == "[DONE]":
            // B01-23: 流正常结束
            INSERT INTO provider_usage_logs(success = TRUE, response_tokens = count_tokens(accumulated_text))
            RETURN StreamEnd
          
          data = parse_json(data_str)
          delta = adapter.convert_stream_chunk(data)
          accumulated_text += delta.content
          YIELD delta
      
      on_error: FUNCTION(error):
        // B01-24: 流中断处理
        LOG.warn("Stream interrupted", error)
        
        // B01-25: 保存已接收部分
        INSERT INTO provider_usage_logs(
          provider_id, model_id, user_id,
          response_tokens = count_tokens(accumulated_text),
          success = FALSE,
          error_code = "stream_interrupted"
        )
        
        RETURN Err(E_STREAM_INTERRUPTED, accumulated_text)
    }
  
  CATCH (error):
    RETURN Err(E_NETWORK_ERROR)
```

#### 分支到测试追踪表

##### 分支编号说明（2026-10-29）

分支 ID 前缀是功能编号（FR-001 → `B01`），见[功能文档规范 §8.3](../../功能文档规范.md#83-分支追踪规则)。本文原有两处编号冲突：§10.2 的流式流程使用了 FR-002 的 `B02-01～B02-08`（FR-002 已将其保留给旧规格），§7.2 适配器又把 `B01-01～B01-10` 用作另一组含义。处理方式如下，旧 ID 不再在本功能中使用：

- 本节同步流程的 `B01-01～B01-11` 保持不变（风险说明与检查清单引用的是这一组）。
- `B01-12～B01-17` 已被[归档的旧 AC 追踪表](../../../99-历史归档/V1-追踪矩阵/2026-10-28-旧AC追踪表.md)占用，跳过不用。
- 本节流式流程：原 `B02-01～B02-08` → `B01-18～B01-25`（按原顺序一一对应）。
- §7.2/§7.3 适配器：原 `B01-01～B01-10` → `B01-26～B01-35`（按原顺序一一对应）。
- 下一个可用编号原为 `B01-36`。2026-10-29 三方兼容协议扩展在[技术设计](01-Provider适配框架-技术设计.md)中分配了 `B01-36～B01-71`，下一个可用编号为 `B01-72`。代码、测试中没有引用这些分支 ID。

| 分支编号 | 分支描述 | AC 映射 | E2E 映射 | NFR 映射 | 风险级别 |
|---------|---------|---------|---------|---------|---------|
| B01-01 | 查询 Provider 配置 | AC01, AC12 | V1-E2E-01 | - | 低 |
| B01-02 | 构造适配器与请求 | AC05 | V1-E2E-01 | - | 低 |
| B01-03 | 读取重试配置 | AC09 | - | - | 低 |
| B01-04 | 重试循环主逻辑 | AC09, AC10 | - | V1-NFR-001 | 🔴 高 |
| B01-05 | 检查成功响应 | AC05 | V1-E2E-01 | - | 低 |
| B01-06 | 记录使用日志 | AC14 | - | V1-NFR-002 | 低 |
| B01-07 | 401 错误处理 | AC02 | - | - | 中 |
| B01-08 | 429 速率限制重试 | AC09 | - | V1-NFR-001 | 🔴 高 |
| B01-09 | 5xx 服务器错误重试 | AC09, AC10 | - | V1-NFR-001 | 🔴 高 |
| B01-10 | 其他错误处理 | AC05 | - | - | 中 |
| B01-11 | 网络错误重试 | AC09 | - | V1-NFR-001 | 🔴 高 |
| B01-18 | 查询 Provider（流式） | AC12 | - | - | 低 |
| B01-19 | 构造流式请求 | AC06 | - | - | 低 |
| B01-20 | 发起 SSE 调用 | AC06 | V1-E2E-01 | - | 中 |
| B01-21～B01-22 | 流处理与 SSE 解析 | AC06 | V1-E2E-01 | V1-NFR-003 | 中 |
| B01-23 | 流正常结束 | AC06 | - | - | 低 |
| B01-24～B01-25 | 流中断保存已接收 | AC07 | - | V1-NFR-001 | 🔴 高 |

**高风险分支说明**：
- 🔴 **B01-04**: 重试循环是可靠性核心，必须充分测试边界（0次、1次、3次重试）
- 🔴 **B01-08, B01-09, B01-11**: 各种错误的重试逻辑，容易出现死循环或过早放弃
- 🔴 **B01-24～B01-25**: 流中断时必须保存已接收部分，否则用户看到的内容会丢失

**NFR 映射说明**：
- **V1-NFR-001 (可靠性)**: 重试机制、错误恢复
- **V1-NFR-002 (可观测性)**: 使用日志记录
- **V1-NFR-003 (性能)**: 流式响应性能

---

### 10.3 验证检查清单

开发完成后，必须通过以下检查才能合并：

**代码检查**：
- [ ] 所有 AC 的目标测试文件已创建
- [ ] 高风险分支（B01-04, B01-08, B01-09, B01-11, B01-24～B01-25）有独立测试用例
- [ ] 重试逻辑有完整的单元测试（0次、1次、3次、边界条件）
- [ ] 流中断场景有明确的回归测试

**测试检查**：
- [ ] 后端集成测试通过率 ≥ 80%（mock 外部 API 的场景可延后）
- [ ] 并发测试验证无竞态条件（5个并发、100个并发）
- [ ] E2E 测试覆盖完整用户流程（V1-E2E-01）

**数据检查**：
- [ ] 使用记录正确保存到数据库（包含 tokens、latency）
- [ ] API Key 加密存储验证通过（bcrypt 或 AES）
- [ ] 模型列表缓存正确（首次同步、手动刷新）

**文档检查**：
- [ ] 验证记录章节已更新（记录日期、通过率、覆盖说明）
- [ ] 所有高风险分支的测试结果已记录

## 11. 风险与依赖

### 11.1 技术风险

| 风险 | 概率 | 影响 | 缓解措施 |
| --- | --- | --- | --- |
| OpenAI API 变更 | 中 | 高 | 版本锁定，监控 API 变更 |
| 流式输出不稳定 | 中 | 中 | 完善错误处理，保存已接收部分 |
| 并发性能瓶颈 | 低 | 中 | 压力测试，必要时引入连接池 |
| API Key 泄漏 | 低 | 高 | 加密存储，日志脱敏 |
| 三方服务偏离协议（三方兼容协议扩展） | 高 | 中 | 通用画像只发标准字段，响应宽容读取（§2.4.7）；已知差异写成内置画像并登记来源与核实日期；参数降级重试最多一次；按服务分别做真实 smoke；偏差改画像数据，不在适配器里加特判 |
| 三方端点把请求引向其他主机或窃取凭据 | 中 | 高 | 不跟随重定向；凭据头只发往 base_url 的主机；画像按完整域名标签匹配且不能改变目标主机；scheme 与明文 HTTP 策略（AC32、AC34） |
| Anthropic API 版本或事件格式变化 | 中 | 中 | 固定 `anthropic-version`；未知流式事件忽略并记录类型名；golden fixture 回归 |
| 思考块签名丢失导致工具循环被上游拒绝 | 中 | 高 | 工作台保留整条 assistant 消息原样回传（技术设计 B01-66），AC24 覆盖 |

### 11.2 外部依赖

| 依赖 | 类型 | 风险 | 应对 |
| --- | --- | --- | --- |
| OpenAI API | 外部服务 | 可用性依赖 | 用户自己配置，支持多 Provider |
| Anthropic Messages API | 外部服务（多协议扩展） | 协议版本与事件格式变化 | 固定协议版本；golden fixture 回归 |
| 三方兼容服务 | 外部服务（三方兼容协议扩展） | 兼容程度不一 | 官方画像表达已知差异，通用画像保守；按服务记录 smoke |
| reqwest | Rust Crate | 版本兼容 | 锁定版本 |
| tokio | Rust Crate | 版本兼容 | 锁定版本 |

### 11.3 功能依赖

| 依赖功能 | 状态 | 影响 |
| --- | --- | --- |
| 本地用户管理（V1-FR-008） | 非前置（D023） | 当前使用宿主派生的 `default-user`；完整多用户隔离随 FR-008 验收 |
| SQLite 数据库 | 迁移依赖 | Provider 相关表由 migration 建立，应用启动时自动迁移 |

## 12. 实现状态

只在 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md) 维护功能状态；本节不复制进度或百分比。

## 13. 变更记录

| 日期 | 变更内容 | 变更人 |
| --- | --- | --- |
| 2024-01-XX | 初始版本，基于 spec-docs-pro 创建 | Kiro |
| 2026-10-29 | §8 按 D038 说明与错误码文档的关系；解决分支 ID 冲突（流式 B02-01～08 → B01-18～25，适配器 B01-01～10 → B01-26～35）；§11.3/§12 删除与 D023 及实现状态矛盾的“规划中/未开始”表述 | Kiro |
| 2026-10-29 | 三方兼容协议扩展草案（D-待编号-A、D-待编号-B、ADR-0003）：新增 §2.4、§3.5～§3.7、§4.3、§6.3、§7.4、§8.1，AC15～AC37 与测试设计，新建技术设计（B01-36～B01-71）；风险等级统一为高风险（原 frontmatter 写“标准”、功能卡写“中等”）；AC01～AC14 原文不变。范围经用户澄清收窄：用户只填协议、base_url 和 API Key，模型自动分类，使用场景留给后续设计，因此“厂商预设”改为内置兼容画像，去掉统一模型选择器与能力拦截（AC27、AC28 的含义随之重定义；这两条和 B01-44、B01-45、B01-67 此前只是未提交的草案，没有被代码、测试或其他文档引用）；补充既有 OpenAI 兼容实现的缺口（§2.4.7、AC34～AC37） | Kiro |
| 2026-10-29 | 厂商改为端点列表并新增“自定义”厂商（用户确认“先选厂商、再选兼容适配器”）：`ProviderVendor` 增加 `endpoints`，校验改为“该厂商有对应端点”，旧数据按主机推断厂商，官方画像按端点的官方主机生效；首批为 OpenAI、Anthropic、自定义，建议第一个扩展 DeepSeek（待真实验证）；§2.4.6 第 13～15 项、AC15、AC16、接口 §6.3 同步 | Kiro |
| 2026-10-29 | 用户澄清“厂商”就是选择适配器，要求保留厂商选择并先提供 OpenAI、Anthropic 两个：恢复厂商表（`vendor_id` 记入 `config`，缺省由 `provider_type` 推断），选厂商即选适配器并预填 base_url，官方画像只在官方主机生效；新增只读命令 `list_provider_vendors`；AC16 含义扩展为厂商选择、画像匹配与高级选项校验；§2.4.7 新增缺口 15 | Kiro |

---

**文档状态**：持续维护；实现状态以 V1 实现状态为准
