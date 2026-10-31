---
title: "01 Provider适配框架 - 技术设计"
version: V1
feature_id: V1-FR-001
updated: 2026-10-29
---
# 01 Provider适配框架 - 技术设计

本文只覆盖 2026-10-29 三方兼容协议扩展（AC15～AC37，分支 B01-36～B01-71），状态为**草案**：方向由 D062 与 [ADR-0003](../../../06-决策记录/ADR/0003-Provider协议适配层与规范化消息模型.md) 确定，口径登记为 D063（评审结果待定）。原有分支 B01-01～B01-35 仍在[主规格 §7.3 与 §10.2](01-Provider适配框架.md#102-技术设计与分支追踪)。需求、接口、数据和 AC 以主规格为准，本文只写如何实现。

来源：[需求](01-Provider适配框架.md#24-三方兼容协议扩展草案) · [接口](01-Provider适配框架.md#63-三方兼容协议扩展的命令与字段草案) · [数据](01-Provider适配框架.md#43-多协议扩展的数据变化草案) · [验收](01-Provider适配框架.md#51-ac-逐项测试设计表) · [错误码](01-Provider适配框架.md#81-多协议扩展拟新增错误码草案)

## 风险与对策

带“现有代码”字样的行是 2026-10-29 阅读代码和厂商文档发现的问题，没有用测试复现；完整缺口清单见主规格 [§2.4.7](01-Provider适配框架.md#247-既有-openai-兼容实现的补齐)。它们影响新协议的实现，在实现切片 S0 中修复，状态与证据只回写到 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)。

| 风险 | 严重程度 | 对策 | 关联决策 |
| --- | --- | --- | --- |
| `adapters/openai.rs` 的 SSE 按网络包解析：每个包只取第一个 `data:` 行，跨包事件不缓冲，解析失败静默产生空块 | 高 | 独立的字节缓冲解码器，按空行切分事件，多行 `data:` 拼接（B01-59） | D063 |
| `retry.rs` 除 401/ProviderNotFound 外全部重试（含 400、404、JSON 解析失败），耗尽后统一改写为 1001008 | 高 | 按错误类别决定是否重试；不可重试的错误保持原码（B01-55～56） | D063 |
| reqwest 60 秒总超时作用于流式响应体 | 高 | 连接、非流式总期限、流式空闲三层超时（B01-57～58） | D063 |
| 流式路径不写使用记录，也没有请求 `include_usage` | 中 | 流结束时只写一次使用记录（B01-63） | - |
| 现有代码：分类算出后不落库，读取时 `model_type` 写死为 `chat`，非对话模型被当作对话模型 | 中 | 分类规则表加 `kind` 持久化，宿主重算（B01-45、B01-67） | D014 |
| 现有代码：`create_provider_with_models` 信任前端提交的模型数据（含主键和能力） | 高 | 只采用 `model_id` 与 `model_name`，其余由宿主重算（B01-67） | AGENTS.md §3 |
| 现有代码：默认重定向策略，请求可能被带到其他主机 | 中 | 不跟随重定向，3xx 一律 `provider_response_invalid`（B01-68） | - |
| 现有代码：`usage` 字段必填，缺字段被当作网络错误并被重试 | 中 | 宽容读取（B01-69）；解析失败不重试（B01-70） | - |
| 现有代码：创建 Provider 必须 `GET /models` 成功；401 被报成 `provider_unavailable` | 中 | 404/405 转探测，401 映射 `invalid_api_key`（B01-39～40、B01-54） | - |
| 现有代码：没有检查 URL scheme | 中 | 保存和调用时都执行端点策略（B01-37） | - |
| 现有代码：工具调用固定带 `tool_choice` 与 `parallel_tool_calls`，`max_tokens` 字段名固定 | 中 | 画像控制是否发送，被点名拒绝时降级重试一次（B01-71） | - |
| 思考块签名丢失；或兼容厂商要求回传的 `reasoning_content` 丢失 | 高 | 工作台保留整条 assistant 消息（B01-66）；各轮 system 与工具定义不变（Anthropic 5.x 在两者变化时使旧思考块失效） | ADR-0003 |
| 默认开启思考的兼容厂商（如 DeepSeek）在工具循环中要求回传 `reasoning_content`，现有 `ToolMessage` 不保存该字段 | 高 | 按上一轮响应里实际出现的推理字段名原样回传（B01-51），`ToolMessage.reasoning` 保存原值；按厂商文档，现有 `openai_compatible` 工具循环可能已在第二轮被拒，未用真实调用复现 | ADR-0003 |
| 三方服务偏离协议（用户自填 base_url，范围无法穷举） | 高 | 通用画像只发标准字段、响应宽容读取；已知差异写成内置画像（B01-44）；按服务做真实 smoke | ADR-0003 |
| 推理内容进入持久化或日志 | 中 | `ToolMessage.reasoning` 不序列化给 renderer，不写库；审计只记类别 | D023 |

## 单元契约

| 函数 | 前置条件 | 返回 / 后置条件 | 副作用 | 幂等 |
| --- | --- | --- | --- | --- |
| `validate_provider_config(config) -> Result<NormalizedConfig>` | 无 | 协议合法、高级选项取值合法、端点合规、base_url 已规范化 | 无 | 是 |
| `list_vendors() -> Vec<VendorInfo>` | 无 | 返回内置厂商表的公开字段（`vendor_id`、显示名、端点列表：适配器、端点名、默认 base_url） | 无 | 是 |
| `infer_vendor(provider_type, base_url) -> VendorId` | 旧数据没有 `vendor_id` | 主机属于某厂商该适配器端点的官方主机则返回该厂商，否则返回 `custom` | 无 | 是 |
| `resolve_profile(vendor, provider_type, base_url) -> Profile` | `vendor` 与 `provider_type` 已校验为有对应端点 | base_url 的主机属于该端点的官方主机（完整域名标签）时返回官方画像，否则返回该适配器的通用画像 | 无 | 是 |
| `classify_model(protocol, model_id) -> (kind, kind_source)` | 无 | 纯函数；anthropic 一律 `text-generation`；其余按规则表，未命中归为 `text-generation` | 无 | 是 |
| `test_connection(config, probe_model?) -> Result<bool>` | 配置已校验 | 模型列表或探测成功时为 true；失败带诊断类别 | 外部请求；探测消耗少量 token | 否 |
| `list_remote_models(adapter) -> Result<Vec<Model>>` | 无 | 全部分页合并，宽容读取，每项已分类 | 外部请求 | 是（只读） |
| `add_manual_model(provider, model_id, kind?) -> Result<Model>` | 当前用户拥有该 Provider | `source=manual` 行存在 | 写 `models`，递增 `config_revision` | 是：`(provider_id, model_id)` |
| `recompute_models(protocol, submitted) -> Result<Vec<Model>>` | 无 | 只采用 `model_id`、`model_name`，主键、`kind`、能力由宿主生成 | 无 | 是 |
| `call(provider, model, request) -> Result<ChatResponse>` | Provider 已启用且 revision 匹配 | 归一化响应 | 外部请求；一条使用记录 | 否 |
| `stream(provider, model, request, cancel) -> Stream<Event>` | 同上 | 有序事件，最多一个终止事件 | 外部请求；流结束时一条使用记录 | 否 |

## 伪代码

外部调用都在数据库事务之外；`credentials_lock` 只在读取快照时持有，网络期间不持有（沿用 D023）。

### 配置、连接测试与模型

```text
FUNCTION validate_provider_config(config) -> Result<NormalizedConfig>:
  IF config.provider_type NOT IN {openai_compatible, anthropic}:
    RETURN Err(unsupported_provider_type)                          // B01-36
  vendor = config.vendor_id ? VENDORS.get(config.vendor_id)
                            : VENDORS.get(infer_vendor(config.provider_type, config.base_url)) // B01-36（旧数据按主机推断，推断不出为 custom）
  IF vendor IS NONE OR vendor.endpoint(config.provider_type) IS NONE:
    RETURN Err(unsupported_provider_type)                          // B01-36（厂商不存在，或该厂商没有这个适配器的端点）
  IF config.auth_style 存在 AND NOT IN {bearer, x-api-key, both}
     OR config.allow_insecure_http 存在 AND 不是布尔值:
    RETURN Err(request_not_supported)                              // B01-36（config 中其他键不动）
  url = parse(config.base_url) OR RETURN Err(insecure_endpoint)
  IF url.scheme NOT IN {http, https}: RETURN Err(insecure_endpoint) // B01-37（file:// 等）
  IF url.scheme == "http" AND NOT (is_loopback(url.host) OR is_private_literal_ip(url.host))
     AND NOT config.allow_insecure_http:
    RETURN Err(insecure_endpoint)                                  // B01-37（调用前对已保存配置同样执行）
  // 私网字面 IP：10/8、172.16/12、192.168/16、fc00::/7、链路本地；主机名（如 ollama.lan）不解析 DNS，按公网处理
  url = trim_trailing_slash(url)
  IF config.provider_type == anthropic AND url.path ends with "/v1":
    url = strip_suffix(url, "/v1")                                 // B01-38（§2.4.6 第 2 项）
  RETURN Ok(config with base_url = url, vendor = vendor)

FUNCTION resolve_profile(vendor, provider_type, base_url) -> Profile:
  host = base_url.host.to_lowercase()
  endpoint = vendor.endpoint(provider_type)
  FOR domain IN endpoint.official_hosts:                           // custom 的端点没有官方主机
    IF host == domain OR host.ends_with("." + domain): RETURN endpoint.official_profile // B01-44（必须在 "." 边界）
  RETURN GENERIC[provider_type]                                    // B01-44（三方兼容服务、custom 和任何未识别主机）
  // 画像只返回请求形状数据；不返回主机、不返回凭据、不放宽 HTTPS

FUNCTION test_connection(config, probe_model?) -> Result<bool>:
  cfg = validate_provider_config(config)?
  profile = resolve_profile(cfg.vendor, cfg.provider_type, cfg.base_url)
  IF profile.models_endpoint != none:
    result = list_remote_models(adapter(cfg))                      // B01-39
    IF result.ok: RETURN Ok(true)
    IF result.status IN {404, 405}: GOTO probe                     // B01-39（没有模型列表接口）
    RETURN Err(diagnose(result))                                   // 401/403/非 JSON/3xx/网络，见主规格 §2.4.5
  probe:
  IF probe_model IS NONE: RETURN Err(request_not_supported)        // B01-40（界面据此要求填写模型名）
  request = {messages: [user "ping"], max_tokens: 16}              // 任何 2xx 都算成功
  RETURN Ok(adapter(cfg).call(probe_model, request).is_ok())       // B01-40（不写使用记录）

FUNCTION list_remote_models(adapter) -> Result<Vec<Model>>:
  IF adapter.protocol == anthropic:
    models = []; after = NONE; pages = 0
    LOOP:
      page = GET {base}/v1/models?limit=100&after_id={after}       // B01-41
      models += page.data
      IF NOT page.has_more OR pages >= MAX_MODEL_PAGES: BREAK      // B01-41（上限防止死循环）
      after = page.last_id; pages += 1
  ELSE:
    models = GET {base}/models .data                               // B01-41（单次返回；画像声明分页时按画像分页，同样受 MAX_MODEL_PAGES 限制）
  models = tolerant_model_list(models)                             // B01-69：丢弃缺 id、id 超过 255 字符的项，按 id 去重取第一项，总数超过 MAX_MODELS 时截断
  RETURN models.map(m ->
    (kind, kind_source) = classify_model(adapter.protocol, m.id)   // B01-45
    m with kind, kind_source, source = remote,
           capabilities = hints(kind, m),                          // 只来自 kind 与协议文档化字段，见“模型分类规则”
           context_window = documented(m, "max_input_tokens"),     // 仅取协议文档化字段，没有则为空
           max_output_tokens = documented(m, "max_tokens"))

FUNCTION recompute_models(protocol, submitted) -> Result<Vec<Model>>:
  FOR s IN submitted:
    IF NOT valid_model_id(s.model_id): RETURN Err(request_not_supported)  // B01-67（整个创建失败）
  RETURN submitted.dedup_by(model_id).map(s ->
    (kind, kind_source) = classify_model(protocol, s.model_id)
    Model{id = new_uuid(), model_id = s.model_id, model_name = s.model_name OR s.model_id,
          kind, kind_source, source = remote, capabilities = hints(kind, none),
          context_window = none, max_output_tokens = none})        // B01-67（不采用前端的主键、分类、能力、长度）

FUNCTION classify_model(protocol, model_id) -> (kind, kind_source):
  IF protocol == anthropic: RETURN (text-generation, protocol)     // B01-45
  id = normalize(model_id)                                         // 小写；去掉最后一个 "/" 之前的路径前缀
  FOR rule IN KIND_RULES IN ORDER:                                 // 顺序见“模型分类规则”
    IF rule.matches(id): RETURN (rule.kind, rule)                  // B01-45
  RETURN (text-generation, default)                                // B01-45

FUNCTION refresh_models(provider):
  ticket = begin_model_refresh(provider)                           // 沿用 D023 刷新代次
  remote = list_remote_models(adapter_for_snapshot(ticket))?
  BEGIN TRANSACTION
    IF ticket 已过期: ROLLBACK; RETURN Err(provider_configuration_changed)
    DELETE models WHERE provider_id = ? AND source = 'remote'      // B01-42
    INSERT remote rows，跳过与 manual 行同 model_id 的项             // B01-42
    config_revision += 1
  COMMIT

FUNCTION add_manual_model(provider, model_id, kind?) -> Result<Model>:
  IF NOT valid_model_id(model_id) OR (kind AND kind NOT IN KINDS): RETURN Err(request_not_supported)
  (k, ks) = kind ? (kind, manual) : classify_model(provider.protocol, model_id)
  BEGIN TRANSACTION
    existing = SELECT models WHERE provider_id = ? AND model_id = ?
    IF existing: COMMIT; RETURN Ok(existing)                       // B01-43（幂等；已有同 ID 的远程项时不覆盖）
    INSERT models(source = 'manual', kind = k, kind_source = ks, capabilities = hints(k, none))
    config_revision += 1                                           // B01-43
  COMMIT
  // remove_manual_model 只删除 source = 'manual' 的行；不存在时返回成功（B01-43）
```

### 调用前检查与 Anthropic 请求构造

```text
FUNCTION preflight(provider, request, mode) -> Result<Profile>:
  cfg = validate_provider_config(provider)?                        // B01-37（已保存配置也检查）
  profile = resolve_profile(cfg.vendor, cfg.provider_type, cfg.base_url) // B01-44
  // 本地拒绝只有两类：协议上不可能成立；违反所用画像明确声明的约束。
  // 模型的 kind 与能力提示不参与判断：宿主不拦截，上游不接受时按错误归一返回
  IF mode == stream AND request.response_format: RETURN Err(request_not_supported) // B01-46
  IF request.temperature IS SET AND profile.rejects_sampling:
    RETURN Err(request_not_supported)                              // B01-46（仅官方 Anthropic 画像：2026-10-21 全部拒绝）
  IF request.response_format.name 不符合 [A-Za-z0-9_-]{1,64}:
    RETURN Err(request_not_supported)                              // B01-46
  IF request.reasoning AND profile.reasoning_template(request.reasoning) IS NONE:
    RETURN Err(request_not_supported)                              // B01-46（该画像下此档位没有映射）
  // 规范契约没有 tool_choice 字段，宿主只发 auto；画像声明不支持 tool_choice 时不发送该字段
  RETURN Ok(profile)

FUNCTION request_headers(profile, cfg, secret) -> Headers:
  style = cfg.auth_style OR profile.auth_style                     // B01-44
  headers = {}
  IF style IN {bearer, both}:    headers["Authorization"] = "Bearer " + secret
  IF style IN {x-api-key, both}: headers["x-api-key"] = secret
  IF protocol == anthropic:
    headers["anthropic-version"] = profile.version                 // 官方 2026-10-21；通用 2023-06-01
    IF profile.official AND cfg.workspace_id: headers["anthropic-workspace-id"] = cfg.workspace_id
  // 两个鉴权头同等脱敏；凭据头只发往 base_url 的主机，不跟随重定向（B01-68）
  RETURN headers

FUNCTION build_anthropic(model, request, profile) -> Body:
  system = []; turns = []
  FOR msg IN request.messages:
    IF msg.role == system:
      IF turns 非空: RETURN Err(request_not_supported)             // B01-47（§2.4.6 第 1 项）
      system.push(msg.content); CONTINUE
    block = to_blocks(msg)                                         // 见下方工具与思考
    IF turns.last.role == role_of(msg): turns.last.content += block // B01-47（合并同角色）
    ELSE: turns.push({role: role_of(msg), content: block})
  IF turns.first.role != user OR turns.last.role != user:
    RETURN Err(request_not_supported)                              // B01-47（Claude 4.6 起不接受末尾 assistant 预填）
  max_tokens = request.max_tokens OR model.max_output_tokens OR profile.default_max_tokens // B01-47
  IF request.reasoning:
    thinking, output_config.effort ← profile.reasoning_template(request.reasoning) // 见“推理映射”
    IF 模板按预算控制 AND budget >= max_tokens: RETURN Err(request_not_supported) // B01-47
  // 调用方未要求推理时不发送 thinking：5.x 模型默认 adaptive，部分模型拒绝 disabled
  RETURN {model, system: join(system), messages: turns, max_tokens, thinking?, output_config?, tools?, tool_choice?}

FUNCTION to_blocks(msg):
  IF msg.role == assistant:
    blocks = msg.reasoning.opaque（原样）                           // B01-49（必须位于开头）
    IF msg.content: blocks += text(msg.content)
    FOR call IN msg.tool_calls:
      input = parse_json_object(call.function.arguments)
      IF input IS ERR: RETURN Err(request_not_supported)           // B01-48
      blocks += tool_use{id: call.id, name: call.function.name, input}  // B01-48
    RETURN blocks
  IF msg.role == tool:                                             // 映射为 user 角色
    RETURN [tool_result{tool_use_id: msg.tool_call_id, content: msg.content}] // B01-48
  RETURN [text(msg.content)]

// 工具定义：{name, description, input_schema: function.parameters}；
// parallel_tool_calls=false → tool_choice {type: auto, disable_parallel_tool_use: true}  // B01-48
```

### 响应归一、OpenAI 兼容差异与结构化输出

```text
FUNCTION normalize_anthropic(body) -> ChatResponse / ToolMessage:
  FOR block IN body.content:
    text      → content += block.text                              // B01-50
    tool_use  → tool_calls += {id, name, arguments: to_json_string(block.input)} // B01-50
    thinking / redacted_thinking → reasoning.text += 可展示部分;
                                   reasoning.opaque += 原块（含签名）  // B01-50
  finish_reason = MAP(body.stop_reason):                           // B01-50
    end_turn | stop_sequence → stop;
    max_tokens | model_context_window_exceeded → length;
    tool_use → tool_calls;   refusal（HTTP 200，带 stop_details）→ content_filter
    其他值（如 pause_turn，V1 不用服务端工具）→ Err(1000000)，与现有工具响应校验一致，不当作 stop
  input = input_tokens + cache_creation_input_tokens + cache_read_input_tokens（缺省为 0）
  usage = {prompt: input, completion: output_tokens, total: input + output_tokens} // B01-50

FUNCTION build_openai(model, request, profile, downgrades) -> Body:
  body[profile.max_tokens_field] = max_tokens                      // 通用画像 max_tokens；官方 OpenAI 画像 max_completion_tokens
  // 画像声明拒绝采样参数时，冲突已在 preflight 拒绝（B01-46），这里不静默丢弃参数
  IF request.reasoning: body += profile.reasoning_template(request.reasoning) // B01-51
  IF stream AND profile.send_stream_options AND "stream_options" NOT IN downgrades:
    body.stream_options = {include_usage: true}                    // B01-71
  IF tools AND profile.send_tool_choice AND "tool_choice" NOT IN downgrades: body.tool_choice = "auto"
  IF tools AND profile.send_parallel_tool_calls AND "parallel_tool_calls" NOT IN downgrades:
    body.parallel_tool_calls = false                               // B01-71
  FOR assistant msg IN messages:                                   // B01-51
    FOR field IN msg.reasoning.fields:                             // 上一轮响应中实际出现过的推理字段
      msg[field.name] = field.value（原样）                          // 通用规则：上游发出什么字段，下一轮就带回什么字段；没有则不添加
    // 白名单：reasoning_content、reasoning、reasoning_details

FUNCTION normalize_openai(body):
  content = choices[0].message.content OR ""                       // B01-69（choices 为空、content 为空都不报错）
  FOR name IN [reasoning_content, reasoning, reasoning_details]:
    IF choices[0].message[name] 存在:
      reasoning.fields += {name, value}; reasoning.text += 可展示部分  // B01-51
  finish_reason：stop、length、tool_calls、content_filter 原值；function_call → tool_calls；其他 → stop  // B01-69
  usage 缺失 → {0, 0, 0} 并标记“用量未知”                           // B01-69

FUNCTION apply_response_format(protocol, profile, request):
  SWITCH (protocol, profile.structured_output):                    // B01-52
    (openai_compatible, json_schema) → response_format = {type: json_schema, json_schema: {...}}   // 通用画像默认
    (openai_compatible, json_object) → response_format = {type: json_object}   // V1 未实现，待首个只支持 json_object 的厂商加入时再做
                                       + 宿主追加一条包含 schema 和 "JSON" 字样的 system 指令
                                       （OpenAI 与 DeepSeek 都要求消息中提到 JSON 才会输出 JSON）
    (anthropic, native)              → output_config.format = {type: json_schema, schema}
                                       （官方画像：GA，无 beta 头；对象须写 additionalProperties:false）
    (anthropic, instruction)         → 宿主追加 system 指令要求只输出符合 schema 的 JSON  // 通用画像默认
  // 宿主不预判上游是否支持：上游不接受按错误归一返回；返回内容再由 validate_structured 校验（B01-53）
  // 不使用“强制工具调用”模拟结构化输出：Opus 5.5、Sonnet 5.5 等模型在任何请求上都拒绝强制 tool_choice

FUNCTION validate_structured(content, schema) -> Result<Value>:
  value = parse_json(content) OR RETURN Err(structured_output_invalid) // B01-53（含 json_object 返回空内容）
  IF NOT jsonschema.validate(schema, value):
    RETURN Err(structured_output_invalid)                          // B01-53（不重试）
  RETURN Ok(value)
```

### 错误归一、重试与超时

```text
FUNCTION classify(status, error_body, protocol) -> (code, retryable, retry_after):
  // 只读取错误类别字段（OpenAI error.code/type，Anthropic error.type），
  // 不把上游错误正文写入错误消息或日志                              // B01-54
  401            → invalid_api_key,            no
  402            → provider_quota_exhausted,   no
  403            → provider_permission_denied, no
  404            → model_not_found,            no
  400 | 413      → 命中“上下文超长”识别规则 ? context_length_exceeded : provider_request_rejected, no
  409 | 422      → provider_request_rejected,  no
  408 | 504      → timeout,                    yes
  429 且错误类别为额度/花费上限（Anthropic enforced_spend_limit_reached，
      OpenAI credit_balance_exhausted、*_spend_limit_exceeded）→ provider_quota_exhausted, no
  429            → rate_limit_exceeded,        yes, header retry-after（秒，可能缺失）
  500..=503, 505..=599, 529 → upstream_server_error, yes, header retry-after
  300..=399      → provider_response_invalid,  no                  // B01-68（不跟随，也不读 Location）
  200 但响应体超限 / 不是 JSON / 缺少协议必需结构 → provider_response_invalid, no // B01-70
  连接失败/连接超时 → 现有网络错误（1000000）/ timeout, yes
  // “上下文超长”识别规则按协议登记在实现中（错误码字段与消息前缀），有单元测试；
  // OpenAI 是否以 context_length_exceeded 表示，官方页面未找到，实现前按真实错误响应确认  // B01-54
  // 解析失败（B01-70）永远不可重试：base_url 填错时只请求一次就报错

HTTP CLIENT:
  redirect policy = none                                           // B01-68：reqwest 默认会跟随最多 10 次；3xx 当作终态
  响应体读取有上限 8 MiB（模型列表、同步响应）；SSE 单个事件上限 1 MiB     // B01-70

FUNCTION send_with_retry(make_request, policy) -> Result:
  attempt = 0; downgrades = {}
  LOOP:
    result = make_request(downgrades)
    IF result.ok: RETURN result
    IF result.status == 400 AND downgrades.len() == 0:
      p = named_param(result.error)                                // B01-71：只在 400 且错误信息点名白名单参数时
      IF p IN {stream_options, parallel_tool_calls, tool_choice, max_tokens, max_completion_tokens}:
        downgrades.add(p); CONTINUE                                // 每次调用最多降级一次，不计入重试次数，不持久化，不记录上游错误正文
    (code, retryable, retry_after) = classify(result)
    IF NOT retryable: RETURN Err(code)                             // B01-56（保持原码）
    IF attempt >= policy.max_retries:
      RETURN Err(attempt > 0 ? max_retries_exceeded : code)        // B01-56
    delay = min(retry_after OR backoff(attempt), policy.max_delay) // B01-55
    SLEEP(delay); attempt += 1
  // max_tokens 与 max_completion_tokens 的降级是互换：被拒绝的那个换成另一个

TIMEOUTS（§2.4.6 第 3 项）:
  connect      = 10s                                               // B01-57
  non_stream   = 120s 总期限                                        // B01-57
  stream_idle  = 120s：两次收到字节之间的最长间隔；ping 也算收到字节    // B01-58
  stream_total = 不设；用户可取消
  空闲超时 → 终止事件 timeout，保留已收内容，不重试                  // B01-58
```

### 流式

```text
FUNCTION sse_decode(bytes_stream) -> Stream<SseEvent>:
  buffer = []                                                      // 字节缓冲，事件完整后再按 UTF-8 解码
  FOR chunk IN bytes_stream:
    buffer += chunk
    WHILE buffer 中有空行分隔符（\n\n、\r\n\r\n 或 \r\r）:
      raw = take_until_separator(buffer)
      event = {type: "event:" 行, data: 多个 "data:" 行用 \n 拼接}  // B01-59
      跳过以 ":" 开头的注释行                                      // B01-59
      YIELD event
  IF buffer 非空: YIELD Err(stream_interrupted)                     // B01-59（半个事件）

FUNCTION map_openai_stream(event):
  data == "[DONE]"        → done
  choices 为空且有 usage   → 记下 usage（不产生空块）               // B01-60
  delta.content           → delta
  delta[reasoning_content | reasoning | reasoning_details] → reasoning_delta，并记录出现过的字段名  // B01-60
  JSON 无法解析           → Err(stream_interrupted)                // B01-60
  缺少 usage              → 记 0 并标记“用量未知”                    // B01-69

FUNCTION map_anthropic_stream(event):
  message_start                        → 记下输入 usage             // B01-61
  content_block_delta.text_delta       → delta                      // B01-61
  content_block_delta.thinking_delta   → reasoning_delta            // B01-61
  content_block_delta.signature_delta  → 丢弃（流式只用于文本对话，不进入工具循环）
  message_delta                        → 记下 stop_reason；usage 是累计值，取最后一次 // B01-61
  message_stop                         → done
  ping                                 → 只重置空闲计时
  error（HTTP 200 之后，如 overloaded_error）→ Err(stream_interrupted) // B01-62
  未知事件类型或未知块类型（如服务端 fallback 块）→ 忽略，记录类型名（不记内容）

重试边界：连接建立且收到第一个事件之前，按 send_with_retry 处理；
收到第一个事件之后不重试、不重放                                    // B01-62
```

### 使用记录、流式 IPC、工作台与选择器

```text
STRUCT StreamAudit { done: AtomicBool }
FUNCTION finish_once(audit, outcome):
  IF audit.done.swap(true): RETURN                                 // B01-63（只写一次）
  SWITCH outcome:
    Done(usage)       → usage_log(success = true, usage)
    Error(code)       → usage_log(success = false, error_code = code)
    Cancelled | Drop  → usage_log(success = false, error_code = 1001007)
  // Drop 时不能 await：spawn 一个任务写记录                         // B01-63

COMMAND call_provider_stream(window, provider_id, model_id, request, on_event: Channel):
  ctx = host::context(&window, ..)                                 // 身份由宿主派生
  stream_id = uuid()
  REGISTRY.insert(stream_id, {window: window.label, cancel: token}) // B01-64
  SPAWN:
    stream = manager.stream(ctx.user, provider_id, model_id, request, token)
    FOR event IN stream:
      IF on_event.send(event) IS ERR: token.cancel(); BREAK        // B01-65（窗口已关闭）
    REGISTRY.remove(stream_id)
  RETURN {stream_id}

COMMAND cancel_provider_stream(window, stream_id) -> bool:
  entry = REGISTRY.get(stream_id)
  IF entry IS NONE OR entry.window != window.label: RETURN false   // B01-64（不泄露其他窗口的流是否存在）
  entry.cancel.cancel(); RETURN true                               // B01-65（终止事件仍由 finish_once 保证只有一次）

FUNCTION workbench_tool_round(context, response):                  // dgos-workbench/src/tools.rs
  // 现状：用户确认后只按 pending.call 重建 assistant 消息，思考块丢失
  pending.assistant = response.message（整条，含 reasoning.text 与 reasoning.opaque） // B01-66
  用户确认后：context.messages.push(pending.assistant)；再 push 工具结果
  // 各轮 system 与工具定义保持不变，同角色合并规则确定，旧思考块才不会失效
  // reasoning 只在内存 ToolContext 中；不写 workbench_task_tools、任务记录和日志

// 保存路径（B01-67）：create_provider_with_models 调用 recompute_models 后再入库，前端提交的分类、能力、主键、长度一律不采用。
// 选择与限定（哪些场景可用哪些模型）不在本设计内；排序第一的默认值（D012）和不自动切换（D023）不变。
```

## 分支到测试追踪

| 分支 | 功能 AC | 跨功能验收 / NFR |
| --- | --- | --- |
| B01-36 | AC16 | - |
| B01-37 | AC32 | - |
| B01-38 | AC15 | - |
| B01-39～40 | AC17、AC37 | `V1-E2E-01` |
| B01-41 | AC15 | - |
| B01-42～43 | AC17 | - |
| B01-44 | AC16、AC15 | - |
| B01-45 | AC27 | - |
| B01-46 | AC23、AC25、AC26 | - |
| B01-47 | AC18 | - |
| B01-48 | AC22 | `V1-FR-006` 工具流程 |
| B01-49 | AC24 | `V1-FR-006` 工具流程 |
| B01-50 | AC18、AC22 | - |
| B01-51 | AC23 | - |
| B01-52～53 | AC25 | - |
| B01-54 | AC29、AC37 | - |
| B01-55～56 | AC30 | - |
| B01-57～58 | AC31 | - |
| B01-59～60 | AC21 | - |
| B01-61 | AC19、AC23 | - |
| B01-62 | AC20、AC30 | - |
| B01-63 | AC19、AC21、AC33 | - |
| B01-64～65 | AC33 | - |
| B01-66 | AC24 | `V1-FR-006` 工具流程 |
| B01-67 | AC35 | - |
| B01-68 | AC34 | - |
| B01-69 | AC28 | - |
| B01-70 | AC28、AC34、AC37 | - |
| B01-71 | AC36 | - |

分支 B01-44、B01-45、B01-67 在本文早先的草稿中分别是“能力目录查询”“能力预检查”“默认选择”，那一版从未提交，也没有被代码、测试或其他文档引用；2026-10-29 范围收窄后按上表重新定义含义，编号不变。

## 并发控制

| 场景 | 冲突点 | 控制方式 | 失败策略 |
| --- | --- | --- | --- |
| 刷新模型与手动增删模型同时发生 | 同一 Provider 的 `models` 行与 `config_revision` | 沿用 D023 的刷新代次；手动增删也递增 revision，过期的刷新不提交 | 返回 `provider_configuration_changed`（1001012） |
| 多个流式调用并发 | 无共享可变状态 | 每个流有独立 adapter 快照和 `stream_id` | 互不影响 |
| 取消与流自然结束同时发生 | 终止事件与使用记录 | `finish_once` 原子标志 | 后到者不产生事件、不写记录 |
| 流进行中修改或轮换 Provider | 凭据与端点 | 已派发的流使用原快照（D023） | 不中断在途流；下一次调用使用新配置 |

死锁预防：网络调用期间不持有 `credentials_lock`；`REGISTRY` 锁不跨越 await。

## 事务边界

- 范围：模型刷新（删除 remote 行、插入新行、递增 revision）在一个事务内，沿用 D023；手动模型的行写入与 revision 递增在一个事务内。
- 隔离级别：SQLite 默认串行化写，理由：单机桌面，写入量低。
- 失败回滚：任一步失败整个事务回滚；外部调用在事务之外，使用记录写入失败时返回 `audit_warning`（沿用现有语义）。

## 幂等与重试

- 幂等键：手动模型以 `(provider_id, model_id)` 为键；删除不存在的手动模型返回成功。
- 外部调用不幂等：只在 B01-55 列出的类别重试；流式收到第一个事件后不重试、不重放（B01-62）。
- 模块 `providers.call` 与工作台工具调用仍为单次尝试，不使用 Provider 重试（D048、D051 与现有 `call_tools`）。

## 外部调用与补偿

超时见 B01-57～58。超时或中断后上游是否计费未知，宿主记为失败的使用记录，不重放、不补偿。界面提示已由 FR-006 决定（D074）：上游已被调用但没有用量（中断、超时、取消）时，聊天界面显示“结果未知，上游可能已计费”；上游未报告用量时聊天消息的用量存 NULL 而不是 0，界面显示“用量未知”；预校验失败没有任务行，不显示该提示。聊天经 `ProviderManager::stream_provider_bound_cancellable_with_provenance` 调用，调用方为 `BuiltinCaller::Workbench`，`request_id` 等于任务 ID。不需要补偿任务。

## 安全与日志

- 敏感字段：API Key（Bearer 与 `x-api-key`）只在构造请求时从 vault 读取，不进日志与错误消息；推理文本和思考块不进 SQLite、使用记录、审计与日志；上游错误只提取类别字段。
- 权限校验：命令层由 `host::context` 派生用户；Manager 层校验 Provider 归属（D023）；`cancel_provider_stream` 校验发起窗口。
- 兼容画像和分类规则只含公开数据，不含任何密钥；画像不能接触凭据，只返回请求形状。
- 凭据头（`Authorization`、`x-api-key`）只发往 base_url 的主机；不跟随重定向（B01-68）。

## 发布与回滚

- 前置条件：D063 冻结；错误码登记到错误码文档；`models.source`、`kind`、`kind_source`、`max_output_tokens` 追加 migration；接口契约登记新命令。
- 回滚方式：新协议只在用户选择 `anthropic` 时启用。回退到旧版本后，`anthropic` 类型的 Provider 无法调用（旧版本返回 1000000），`openai_compatible` 不受影响；新增的列被旧代码忽略；新形状的 `capabilities` JSON 在旧代码中解析为空能力列表。本次对既有 `openai_compatible` 行为的改动（重试分类、超时、不跟随重定向、宽容读取）没有开关，回退只能回退版本。
- 监控指标：使用记录按 `error_code` 聚合，不新增遥测。

## 协议映射

本节和后两节的厂商事实于 2026-10-29 由助手子任务从官方文档页面（WebFetch 或 curl）核对，没有发起真实 API 调用；标“未核实”的项在第一次真实 smoke 前确认。来源见本节末尾。

| 规范契约 | `openai_compatible` | `anthropic` |
| --- | --- | --- |
| 端点 | `POST {base}/chat/completions` | `POST {base}/v1/messages` |
| 鉴权 | 通用画像 `Authorization: Bearer`，可用 `auth_style` 覆盖 | 官方画像只发 `Authorization: Bearer`（`x-api-key` 是仍受支持的旧方式），通用画像同时发 Bearer 与 `x-api-key`（§2.4.6 第 4 项）；`anthropic-version`：官方 `2026-10-21`，通用 `2023-06-01`；官方画像可选 `anthropic-workspace-id` |
| system | 原样作为消息 | 开头的 system 提升为顶层 `system`；中间出现按 §2.4.6 第 1 项处理 |
| 角色 | 原样 | 连续同角色合并（上游本身也会合并，宿主合并保证确定性）；首条和末条必须是 user |
| `max_tokens` | 通用画像 `max_tokens`，官方 OpenAI 画像 `max_completion_tokens`；被上游点名拒绝时互换一次（B01-71） | 必填；缺省取模型的 `max_output_tokens`，再取画像默认值 |
| `temperature` | 原样发送；官方 OpenAI 画像在推理档位非 `none` 时本地拒绝 | 通用画像原样发送；官方 Anthropic 画像（`2026-10-21` 在所有模型上拒绝）本地拒绝 |
| 工具定义 | 原样 | `{name, description, input_schema}` |
| `tool_choice` | `auto`；画像声明不发送或被上游点名拒绝时去掉 | `{type: auto, disable_parallel_tool_use: true}` |
| 工具调用 | `tool_calls[].function.arguments`（字符串） | `tool_use.input`（对象），宿主互转 |
| 工具结果 | `role: tool` | user 消息开头的 `tool_result` 块，紧跟 `tool_use` 所在轮 |
| 推理请求 | 画像的推理参数模板 | 官方画像：`thinking: {type: adaptive, display: summarized}` 加 `output_config.effort`；通用画像：`thinking: {type: enabled, budget_tokens}` |
| 推理返回 | 白名单字段 `reasoning_content`、`reasoning`、`reasoning_details`，命中哪个记录哪个 | thinking 与 redacted_thinking 块；5.x 模型默认 `display: omitted`，文本为空，块仍须回传 |
| 结构化输出 | `response_format`：`json_schema`（通用画像默认；`json_object` 模式 V1 未实现，见“已知差异”第 3 项） | 官方画像 `output_config.format: {type: json_schema, schema}`；通用画像用 system 指令加宿主校验 |
| 结束原因 | 原值（`function_call` 映射为 `tool_calls`） | 见 B01-50 |
| 用量 | `prompt_tokens`、`completion_tokens`、`total_tokens` | 输入为三项输入字段之和；不返回总数，宿主计算 |
| 流式 | `data:` 行加 `[DONE]`；`include_usage` 时 `[DONE]` 前有一块 `choices: []` 的用量；流被截断时可能没有 | 带类型的事件；`message_delta.usage` 累计；必须忽略未知事件 |
| 错误体 | `{error: {message, type, code}}` | `{type: error, error: {type, message}, request_id}`；HTTP 200 后以 `event: error` 出现 |
| 模型列表 | `GET {base}/models`；画像声明分页时按画像分页 | `GET {base}/v1/models?limit&after_id`，返回 `has_more`、`last_id`；兼容服务可能没有（404 转探测） |

## 推理映射

| 规范 `reasoning` | OpenAI | Anthropic（官方） | 按预算控制的厂商 |
| --- | --- | --- | --- |
| `effort` | `reasoning_effort`，原样交给上游；上游按模型拒绝不支持的档位 | 官方画像 `output_config.effort`：`low`、`medium`、`high`、`xhigh`、`max`；`none`、`minimal` 在该画像下没有映射，本地拒绝（B01-46）。通用画像：`low`/`medium`/`high` 映射为 `budget_tokens` 2048/8192/16384（推断值，需 smoke 校准），其他档位本地拒绝 | 按画像模板映射，例如 DeepSeek `reasoning_effort` 只有 `low`、`high`、`max` |
| `budget_tokens` | 不使用 | 官方画像不使用（`thinking.type: enabled` 加 `budget_tokens` 已弃用，Claude 4.7 起返回 400）；通用画像按预算控制，须小于 `max_tokens`（B01-47） | 百炼 `thinking_budget`、硅基流动 `thinking_budget`（128～32768）、OpenRouter `reasoning.max_tokens` |
| 未设置 | 不发送 | 不发送 `thinking`（5.x 默认 adaptive；部分模型拒绝 `disabled`） | 按服务默认（多家默认开启思考） |

本地拒绝只发生在画像没有该档位映射时（B01-46）；`openai_compatible` 的档位原样交给上游，宿主没有按模型维护档位子集的目录（不做能力拦截）。不就近替换为其他档位。

## 厂商表与兼容画像

用户添加 Provider 时先选厂商；厂商下有一个或多个端点，端点才对应适配器、默认 base_url、官方主机和官方画像（主规格 §2.4.2）。厂商表是宿主内置数据，界面通过 `list_provider_vendors` 读取。首批三个厂商，只有“自定义”有两个端点；以后新增厂商是往下表加行，复用适配器。

| 厂商 | `vendor_id` | 端点（适配器） | 默认 base_url | 官方主机 | 官方画像 |
| --- | --- | --- | --- | --- | --- |
| OpenAI | `openai` | OpenAI 兼容（`openai_compatible`） | `https://api.openai.com/v1` | `api.openai.com` | 下表“OpenAI 官方” |
| Anthropic | `anthropic` | Anthropic 兼容（`anthropic`） | `https://api.anthropic.com` | `api.anthropic.com` | 下表“Anthropic 官方” |
| 自定义 | `custom` | OpenAI 兼容（`openai_compatible`）；Anthropic 兼容（`anthropic`） | 无，用户填写 | 无 | 无，始终使用通用画像 |

画像由 `resolve_profile(vendor, provider_type, base_url)` 选择（B01-44）：base_url 的主机属于该端点的官方主机（完整域名标签，`.` 边界）时用官方画像，否则用该适配器的通用画像。接入 Ollama、LM Studio、vLLM、各类网关和暂时没有内置条目的服务（包括 DeepSeek）时，选“自定义”厂商，走通用画像，靠宽容读取（B01-69）和参数降级（B01-71）适应。用 OpenAI 厂商改写 base_url 去连其他服务不会报错，结果相同，但界面不推荐。

| 画像 | 生效条件 | 适配器 | 鉴权与版本头 | 与通用画像的差异 |
| --- | --- | --- | --- | --- |
| OpenAI 官方 | OpenAI 厂商且主机为 `api.openai.com` | `openai_compatible` | Bearer | `max_completion_tokens`；推理档位非 `none` 时本地拒绝 `temperature`、`top_p`（官方文档规则）；结构化输出 `json_schema`；部分模型在 Chat Completions 中不支持工具，不据此拦截 |
| Anthropic 官方 | Anthropic 厂商且主机为 `api.anthropic.com` | `anthropic` | 仅 Bearer；`anthropic-version: 2026-10-21`；可选 `anthropic-workspace-id` | 本地拒绝 `temperature`（`ChatRequest` 目前没有 `top_p`、`top_k`，见“已知差异”第 1 项）；推理 `thinking: adaptive` 加 `output_config.effort`；结构化输出 `output_config.format`；模型列表返回 `capabilities`、`max_input_tokens`、`max_tokens` |
| 通用 `openai_compatible` | 其他所有主机 | `openai_compatible` | Bearer | `max_tokens`；流式带 `stream_options.include_usage`，带工具时发 `tool_choice`、`parallel_tool_calls`，被点名拒绝时降级一次；推理 `reasoning_effort` 原样；结构化输出 `json_schema` |
| 通用 `anthropic` | 其他所有主机 | `anthropic` | Bearer 加 `x-api-key`；`anthropic-version: 2023-06-01` | 推理 `thinking: enabled` 加 `budget_tokens`；结构化输出用 system 指令加宿主校验；模型列表可能没有（404 转探测） |

### 已核对的三方差异（候选厂商，首批不内置）

2026-10-29 由助手子任务从官方文档页面核对，没有真实调用。当前这些服务由用户选“自定义”厂商接入，走通用画像加宽容读取；要把某个服务提升为内置厂商时，下表对应一行就是该厂商条目的初稿（各端点的默认 base_url、官方主机、官方画像），仍复用现有适配器，并同时确定官方画像的生效范围（主规格 §2.4.6 第 14 项）。建议第一个加入 DeepSeek：它有两个端点，OpenAI 兼容 `https://api.deepseek.com`（Bearer），Anthropic 兼容 `https://api.deepseek.com/anthropic`（`x-api-key`），加入前先用真实 Key 验证（第 15 项）。

| 服务 | OpenAI 兼容 base_url | 模型列表 | 推理请求 | 推理回传 | 结构化输出 | 其他差异 |
| --- | --- | --- | --- | --- | --- | --- |
| DeepSeek | `https://api.deepseek.com` | 有 | `thinking.type` 加 `reasoning_effort`（`low`、`high`、`max`），默认开启 | `reasoning_content`；**有工具时必须回传历次的值，否则 400** | `json_object` | 思考模式忽略 `temperature`；Anthropic 兼容 `https://api.deepseek.com/anthropic`，`x-api-key` |
| 阿里云百炼 | `https://dashscope.aliyuncs.com/compatible-mode/v1`（新地址含工作区 ID） | 未知 | `enable_thinking` 加 `thinking_budget` | `reasoning_content`，回传可选 | 视模型而定 | Key 绑定地域，跨地域 401；Anthropic 兼容端点没有 `/v1/models` |
| Moonshot（Kimi） | `https://api.moonshot.cn/v1`（国际站 `.ai`） | 有 | K3 `reasoning_effort`；K2.x `thinking.type` | `reasoning_content` 须原样保留；K3 须回传整条 assistant 消息 | `json_schema`、`json_object` | 部分模型不能改 `temperature` |
| 智谱 | `https://open.bigmodel.cn/api/paas/v4`（国际站 `https://api.z.ai/api/paas/v4`） | 未知 | `thinking.type` | `reasoning_content` 须随工具结果回传 | `json_object` | 部分模型不能关闭思考 |
| OpenRouter | `https://openrouter.ai/api/v1` | 有（offset/limit 分页） | `reasoning.effort` 或 `reasoning.max_tokens` | `reasoning_details` 原样回传 | `json_schema`，按上游端点而定 | Anthropic 兼容端点只接受 Bearer |
| 硅基流动 | `https://api.siliconflow.cn/v1`（国际站 `.com`） | 有 | `enable_thinking` 加 `thinking_budget` | `reasoning_content`，是否回传未核实 | `json_object` | - |
| Ollama（本地） | `http://localhost:11434/v1` | 有 | `reasoning_effort` | 返回字段名未核实 | `response_format` 含 schema | 不支持 `tool_choice`；本地主机名无法区分服务，只能走通用画像，靠参数降级处理 |
| LM Studio（本地） | `http://localhost:1234/v1` | 有 | 未核实 | `reasoning_content` 或 `reasoning` | `json_schema` | 同上 |
**来源**（2026-10-29 访问）：

- Anthropic：`platform.claude.com/docs/en/` 下的 `api/overview`、`api/versioning`、`api/messages/create`、`api/errors`、`api/rate-limits`、`api/models/list`、`build-with-claude/streaming`、`build-with-claude/thinking`、`build-with-claude/extended-thinking`、`build-with-claude/structured-outputs`、`build-with-claude/handling-stop-reasons`、`agents-and-tools/tool-use/handle-tool-calls`
- OpenAI：`developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create`、`/api/docs/guides/reasoning`、`/api/docs/guides/latest-model`、`/api/docs/guides/error-codes`
- DeepSeek：`api-docs.deepseek.com`（`guides/thinking_mode`、`guides/anthropic_api`、`api/list-models`、`guides/json_mode`）
- 阿里云百炼：`help.aliyun.com/zh/model-studio/`（`compatibility-of-openai-with-dashscope`、`anthropic-api-messages`、`deep-thinking`、`json-mode`）
- Moonshot：`platform.moonshot.ai/docs/`（`api/chat`、`api/list-models`、`guide/use-thinking-models`）
- 智谱：`docs.bigmodel.cn/cn/guide/develop/openai/introduction`、`docs.z.ai/guides/capabilities/thinking-mode`
- OpenRouter：`openrouter.ai/docs/`（`guides/best-practices/reasoning-tokens`、`api/api-reference/models/list-all-models-and-their-properties`、`guides/features/structured-outputs`）
- 硅基流动：`docs.siliconflow.com/en/`（`api-reference/chat-completions/chat-completions`、`userguide/capabilities/reasoning`）
- Ollama：`docs.ollama.com/api/openai-compatibility`、`/api/anthropic-compatibility`
- LM Studio：`lmstudio.ai/docs/developer/openai-compat`、`/anthropic-compat`

OpenAI 是否以 `context_length_exceeded` 表示上下文超长，在可访问的官方页面中没有找到；“上下文超长”识别规则（B01-54）实现前按真实错误响应确认。

## 模型分类规则

用于 `classify_model`（B01-45），只对 `openai_compatible` 生效；`anthropic` 一律 `text-generation`（`kind_source=protocol`）。规则表是内置数据，由我起草，没有对照真实的三方模型列表验证；首批规则以下方用例表为准，实现时必须让全部用例通过，增补规则必须同时增补用例。

**匹配方式**：把 `model_id` 转小写，去掉最后一个 `/` 之前的路径前缀（`vendor/model`、`models/model`），再做**左边界**匹配：模式必须出现在开头，或紧跟在 `-`、`_`、`.`、`:`、空格之后。这样 `tts` 能匹配 `gpt-4o-mini-tts`，不会匹配 `https` 一类字符串中间的片段。

**顺序**（先命中先返回；顺序本身是规则的一部分）：

| 顺序 | `kind` | 模式（左边界） | 说明 |
| --- | --- | --- | --- |
| 1 | `rerank` | `rerank` | 必须先于嵌入：`bge-reranker-*` 同时含 `bge-` |
| 2 | `embedding` | `embed`、`bge-`、`e5-`、`gte-`、`text-embedding` | `embed` 覆盖 `embedding`、`embeddings`、`nomic-embed-text` |
| 3 | `moderation` | `moderation` | 覆盖 `omni-moderation-*`、`text-moderation-*` |
| 4 | `speech-transcription` | `whisper`、`transcribe`、`asr`、`sensevoice`、`paraformer` | 与顺序 5 的模式目前没有交集，顺序只是保持稳定 |
| 5 | `speech-synthesis` | `tts`、`cosyvoice`、`fish-speech`、`text-to-speech` | 覆盖 `tts-1`、`gpt-4o-mini-tts` |
| 6 | `video-generation` | `sora`、`veo`、`kling`、`runway`、`hailuo`、`cogvideo`、`seedance`、`wan2`、`t2v`、`i2v` | - |
| 7 | `image-generation` | `dall-e`、`dalle`、`gpt-image`、`flux`、`stable-diffusion`、`sdxl`、`sd3`、`midjourney`、`imagen`、`kolors`、`cogview`、`seedream`、`qwen-image` | 不用单独的 `image`：`gpt-4-vision`、`qwen-vl` 等是对话模型 |
| 默认 | `text-generation` | （无命中） | `kind_source=default` |

**能力提示 `hints(kind, m)`**（只供展示，宿主不据此拦截）。`kind` 不是 `text-generation` 时五项一律 `unknown`；以下规则只对 `text-generation` 计算：

| 提示 | 取值来源 |
| --- | --- |
| `streaming` | `supported`（对话接口默认支持流式） |
| `vision` | 左边界模式 `vision`、`vl`、`omni` 命中为 `supported`；否则 `unknown`（不会是 `unsupported`，规则无法证明不支持） |
| `reasoning` | 左边界模式 `o1`、`o3`、`o4`、`r1`、`reasoner`、`thinking`、`qwq` 命中为 `supported`；否则 `unknown`；`anthropic` 协议取上游 `capabilities` 的文档化字段 |
| `tools`、`structured_output` | 一律 `unknown`，除非协议文档化字段明确给出（`anthropic` 的 `/v1/models` 返回的 `capabilities`） |

**用例表**（每行必须有对应单元测试；`kind` 列是期望值，`来源` 列是 `kind_source`）：

| 模型 ID | 期望 `kind` | 来源 | 说明 |
| --- | --- | --- | --- |
| `gpt-4o`、`deepseek-chat`、`qwen2.5-72b-instruct`、`llama-3.1-8b` | `text-generation` | default | 默认归类 |
| `gpt-4o-audio-preview` | `text-generation` | default | 仍走对话接口 |
| `gpt-4-vision-preview`、`qwen-vl-max` | `text-generation` | default | `vision` 提示为 `supported` |
| `text-embedding-3-small`、`bge-m3`、`nomic-embed-text` | `embedding` | rule | - |
| `bge-reranker-v2-m3`、`jina-reranker-v2` | `rerank` | rule | 顺序 1 先于顺序 2 |
| `omni-moderation-latest` | `moderation` | rule | `omni` 不会让它变成对话模型 |
| `whisper-1`、`gpt-4o-transcribe`、`sensevoice-small` | `speech-transcription` | rule | 修正原 `audio-generation` 标签 |
| `tts-1`、`gpt-4o-mini-tts`、`cosyvoice-v1` | `speech-synthesis` | rule | - |
| `dall-e-3`、`gpt-image-1`、`flux-dev`、`stable-diffusion-xl` | `image-generation` | rule | - |
| `sora-2`、`wan2.1-t2v` | `video-generation` | rule | - |
| `openrouter/auto`、`meta-llama/llama-3-70b` | `text-generation` | default | 路径前缀被去掉后再匹配 |
| `scotts-chat`（构造用例） | `text-generation` | default | `tts` 出现在词中间（`scotts`），不在左边界，不能误判为语音合成 |

## 实现与规格的已知差异

2026-10-30 按代码核对。这里记录事实，不改写冻结决策原文（D062、D063 评审结果待定）。第 1～4 项已在 2026-10-30 按下方“处理结果”处理，仍需评审时确认；第 5～8 项是事实记录。

| # | 规格或设计的说法 | 代码的实际行为 | 影响与待决事项 |
| --- | --- | --- | --- |
| 1 | “官方 Anthropic 画像本地拒绝 `temperature`、`top_p`、`top_k`”（§2.4.6 第 7 项、AC26、上文画像表） | `ChatRequest` 只有 `messages`、`temperature`、`max_tokens`、`stream`、`reasoning`、`response_format`，没有 `top_p`、`top_k`。官方画像只拒绝 `temperature`。调用方发来的 `top_p`、`top_k` 在反序列化时被丢弃（结构体没有 `deny_unknown_fields`），既不发往上游，也不报错 | 规格里“拒绝 `top_p`/`top_k`”的那一半目前没有可测的路径。选项：(a) 规格改为“只有 `ChatRequest` 里存在的采样参数才受此约束”；(b) 给 `ChatRequest` 增加这两个可选字段，随之要在两个适配器、官方画像拒绝逻辑和测试里实现，并且模块 `providers.call` 的请求字段随之变化。丢弃而不报错这一点也需要确认 |
| 2 | Anthropic 的 `max_tokens` 取“请求值，其次模型的 `max_output_tokens`，再次画像默认值”，没有给默认值的数字 | 画像默认值是 4096（`vendor.rs` 的 `ANTHROPIC_DEFAULT_MAX_TOKENS`），是实现选择，规格和官方文档都没有给数 | 4096 可能截断较长的回答，调用方可显式给 `max_tokens`。需要评审确认或改值；没有用真实 Anthropic 官方服务校准过（2026-10-30：不传 `max_tokens` 的请求在一个第三方网关上被接受，回复很短，没有测截断；见 §2.4.6。同日补充：这个网关的模型行没有 `max_output_tokens`，调用方不传 `max_tokens` 时实际用的就是 4096） |
| 3 | 画像字段“结构化输出方式”和上文映射表列出 `json_schema` 或 `json_object` 两种 | 只实现了 `json_schema`。`response_format.type` 不是 `json_schema` 时 `check_format` 返回 `request_not_supported`，上游 0 次请求。`json_object` 模式没有实现，伪代码里 `(openai_compatible, json_object)` 分支没有对应代码 | 只支持 JSON Schema 的服务可用；只支持 `json_object` 的服务（候选表里的 DeepSeek、智谱、硅基流动）暂时不能用结构化输出。需要决定：规格去掉 `json_object`，或补实现 |
| 4 | 规格没有规定工具调用响应的结束原因与内容矛盾时的错误码（AC22 备注） | 两个适配器的 `parse_tool_response` 在结束原因不是 `stop`/`tool_calls`，或 `stop` 带工具调用，或 `tool_calls` 不带工具调用时，返回 `InternalError`（1000000）；`openai_compatible` 另在 `choices` 不止一项、角色不是 `assistant`、带 `tool_call_id` 时同样返回 1000000。任务错误码因此是 `provider_1000000` | 这是上游响应异常，不是宿主内部错误，1000000 不准确。候选：归为 `provider_response_invalid`（1001025）。改码会影响工作台任务错误码和 `tool_flow` 测试里只断言前缀 `provider_` 的用例 |
| 5 | “失败的调用记 0 token” 的一般口径 | `structured_output_invalid` 失败时，上游已回答并可能计费，使用记录写上游真实返回的输入、输出 token（`ProviderAdapter::call_with_usage`，`CallFailure.usage`）。其他失败仍记 0。上游没有返回用量时仍记 0 | 这是用户在本轮指定的取舍。`call_tools` 路径不校验结构化输出，不受影响；流式不支持 `response_format` |
| 6 | AC31 “连接超时 10 秒” | 默认值由 `AdapterOptions.connect_timeout` 提供；测试用 300 ms 的注入值，对接受 TCP 连接但不回应 TLS 握手的本机监听器验证。验证的是“连接阶段（含 TLS 握手）超时”，不是“TCP 三次握手无响应”：后者需要会丢弃 SYN 的网络环境，macOS 本机构造不出 | AC31 的连接超时证据是握手阶段，不覆盖 SYN 黑洞场景 |
| 7 | AC09 “间隔 1s/2s/4s” | 策略层用 tokio 虚拟时间验证，3 次重试的等待恰好是 1 s、2 s、4 s，共 4 次尝试，最终 1001008。适配器层只能用真实时间的缩小版（基准 100 ms）验证次数和“不短于 100/200/400 ms”：下限能拒绝固定 100 ms 的等待，但上界 +500 ms 的容差宽于相邻档位的间隔，固定约 400 ms 的等待也能通过，所以它不能证明倍增 | 精确的 1/2/4 秒只在策略层有证据；适配器层只证明接线（4 次请求、等待不短于各档下限） |
| 8 | 规格只给出连接、模型列表、请求、流空闲四个超时（§2.4.6 #3），没有说明 HTTP 客户端何时创建 | 原先 `ProviderManager::create_adapter` 每次调用都经 `Transport::new` 新建 `reqwest::Client`（原生 TLS），构建在任何超时之外。FR-006 阶段 6 的一次性探针在 macOS 上测得 40 次构建中 1 次约 3.1 秒、其余 0～1 毫秒；这是 `timeouts.rs` 等上界计时测试偶发失败的根因（测试已改为先构建再计时）。构建期间持有 `credentials_lock`，取消的实际停止会滞后到构建结束 | 已处理（2026-10-30，用户同意）：`ProviderManager` 在创建时用 `transport::http_client` 构建一个客户端（不跟随重定向、默认 10 秒连接超时），`create_adapter` 经 `with_client` 把它的克隆交给每个适配器，克隆共享连接池。直接构造适配器（测试、示例）仍各自构建。回归测试 `tests/shared_client.rs`：同一管理器两次调用只建立一个 TCP 连接（修复前为 2）。只在一台 macOS 机器上观测，慢构建的原因（代理配置读取或根证书加载）未逐项确认 |

**处理结果（2026-10-30，助手按用户授权选定，评审结果待定）**

1. `top_p`/`top_k`：改规格，不改代码。规格、画像表和 D063 都写明“只有 `ChatRequest` 里存在的采样参数受官方画像拒绝约束”，目前就是 `temperature`。`top_p`、`top_k` 仍被反序列化丢弃、不报错。要支持它们需要先决定是否给 `ChatRequest` 加字段，那会改变模块 `providers.call` 的请求字段，另行决策。
2. `max_tokens` 默认 4096：保留，标注为推断值。调用方（工作台、应用层）都显式传了 `max_tokens`，默认值只影响没传的调用方；等真实调用再校准。
3. `json_object`：改规格，标注 V1 未实现。需要它的厂商（候选表里的 DeepSeek、智谱、硅基流动）加入时再做。
4. 结束原因与内容矛盾：保持 1000000，不改成 1001025。1001025 的含义是“响应不是预期的 JSON、超限或收到 3xx”，这里是合法 JSON 但内容自相矛盾，语义不同；改码还会影响工作台任务错误码和 `tool_flow` 里只断言前缀 `provider_` 的用例。代价是这类上游异常在任务里显示为 `provider_1000000`，和宿主内部错误分不开；以后需要区分时另行登记新码。


## 变更记录

| 日期 | 变更 | 关联决策 |
| --- | --- | --- |
| 2026-10-29 | 初始版本：多协议扩展草案，分支 B01-36～B01-67 | D062、D063、ADR-0003 |
| 2026-10-29 | 厂商表恢复并改为端点列表加“自定义”厂商：`infer_vendor`、`resolve_profile(vendor, provider_type, base_url)`、校验“该厂商有对应端点”；候选厂商建议 DeepSeek 先行（待真实验证）；B01-36、B01-44 的含义随之调整，编号不变 | D062、D063、ADR-0003 |
| 2026-10-29 | 范围收窄后重写：厂商预设改为兼容画像，去掉能力拦截与统一模型选择器，新增模型自动分类、宽容读取、不跟随重定向、参数降级和保存时重算（B01-68～B01-71，B01-44、B01-45、B01-67 重新定义），补充既有 OpenAI 兼容实现缺口；AC15～AC37 | D062、D063、ADR-0003 |
| 2026-10-30 | 记录实现与设计的差异（见“实现与规格的已知差异”）：`top_p`/`top_k` 不在 `ChatRequest`、`max_tokens` 兜底 4096、`json_object` 未实现、工具调用结束原因与内容矛盾返回 1000000、`structured_output_invalid` 的使用记录写上游真实 token。只记录事实，不改冻结决策 | D062、D063 |
| 2026-10-30 | 已知差异第 8 项：每次调用新建 HTTP 客户端，偶发约 3.1 秒构建延迟在超时之外；`timeouts.rs`、`anthropic_stream.rs`、`retry_classification.rs` 的上界计时改为先构建再计时（FR-006 阶段 6 定位） | - |
| 2026-10-30 | 已知差异第 8 项处理：`ProviderManager` 复用一个 HTTP 客户端（`transport::http_client`、`Transport::with_client`、两个适配器的 `with_client`），新增 `tests/shared_client.rs` | - |
