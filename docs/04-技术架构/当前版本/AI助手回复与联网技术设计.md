---
title: AI 助手回复与联网技术设计
version: V1
technical_task: ASSISTANT-WEB-REPLY
spec_status: Ready
risk: L3
updated: 2026-11-11
---
# AI 助手回复与联网技术设计

依据：[D147](../../06-决策记录/V7-冻结决策.md#d147-ai-助手回复执行体验与受控联网)、[FR-006](../../03-功能规格/V1/02-应用模块/06-AI工作台.md)、[产品方案](../../02-产品与版本/AI助手回复区与联网能力改进草案.md)。本技术任务在既有运行基础上实施，不开放任意Shell、后台自治、CLI或模块联网授权；证据集中在[V1实现状态](../../02-产品与版本/当前版本/V1-实现状态.md)。

## 风险与对策

| 风险 | 对策 |
| --- | --- |
| 页面/模型自授联网或越权读历史 | 网络能力固定在本次已准入请求；宿主派生账户、租约和会话；来源随运行归属 |
| DNS重绑定、重定向进入私网 | 公开HTTPS/443、整组DNS答案检查、固定解析结果连接、逐跳复验、禁代理 |
| 查询/密钥泄露 | 密钥只在OS vault与敏感HTTP header；只发送必要查询或URL，无Cookie或登录态；日志固定错误 |
| 凭据清理与配置并发 | 与Provider cleanup共享互斥锁，先outbox后store/readback，指针CAS与ack短事务 |
| 模型伪造来源 | 宿主随机source ID，引用只匹配当前运行真实source；未知引用不可点击 |
| 迟到事件/历史读取回退终态 | 单调seq、运行身份核对、终态封口、只读快照补齐，不执行重放 |
| 跨轮文字混作最终答案 | model_segment明确轮次；先保存有界中间说明，再重置当前正文；不存reasoning |
| 中断误显示远端回滚或自动重发 | MCP unknown保持阻断；只读网页取消单独记失败，不当作已撤销写入 |

## 单元与接口契约

### 联网工具

- `WebAccess {search, fetch}`默认均false，`deny_unknown_fields`；发送、重新生成、继续和预算检查显式携带。它不是会话长期批准，也不进入模块`AiFoundationOptions`。
- `WebService`绑定DB、vault、共享credential mutation lock与账户租约。`status`只读配置引用，不主动触发钥匙串；`configure`通过ChatManager占用同一账户准入槽，避免生成与密钥轮换竞态。
- `WebRuntime`固定账户、请求、能力与搜索凭据引用，注册`dgos_web_search(query)`、`dgos_web_fetch(url)`。每步前后重验租约/配置。只读能力不进入MCP批准槽，MCP仍逐次批准。
- 搜索固定`https://api.search.brave.com/res/v1/web/search`，用户自备独立密钥；不接受前端端点。未配置搜索不准入，fetch独立可用。搜索不静默降级到其他后端。
- 搜索每请求最多3次，读取最多5次，受公共总8次工具操作限制；无自动重试。单网络操作25秒，连接8秒，重定向最多3次；聊天总10分钟。响应体搜索256KiB/网页512KiB，正文提取12KiB，最终工具JSON还受剩余上下文预算约束。拒绝非identity压缩响应，不把压缩炸弹交给自动解压。
- HTML采用无DOM的有界token流提取，避免格式元素隐式重建放大。tokenizer前检查至多4096个字面`<`及单标签8192字节，超限返回`web_html_complexity_limit`；复杂合法网页可能被保守拒绝，不执行脚本、样式或子资源。工具结果使用共享执行器的双层JSON序列化贡献裁剪，不只计算内层正文。联网IPC独立使用`1006022/web_unavailable`及固定reason，不复用工具批准错误。
- 每个WebActivity具有稳定id及running/completed/failed状态，真实运行和结果回调落入`kind=web`的ChatRunStep；失败只有固定错误，无网络错误原文。结果包括宿主生成的sources，不把“已找到”记为全文读取。

### 来源与回复

`WebSource`包含`id,url,title,excerpt,retrieved_at,kind,truncated`，kind区分search_result/web_page。网页正文只在模型上下文有界消费，持久回执只保留必要节选与元数据。保存沿账户聊天域与会话删除，不新增全文网页缓存或外部内容执行。

正文使用`[label](dgos-source:ID)`标记；Markdown只为当前可信运行source集合中存在的ID创建来源预览。未知ID显示不可用；普通HTTPS链接沿既有外链确认，不能自动变为搜索证据。来源标题、URL和节选作为文本显示，不执行HTML。来源属于真实记录不等于证明模型推断正确。

回复内执行视图使用实时步骤与持久快照的同一投影；已完成消息按可见性读取，并发最多2。可见回复持有投影至离开可见区或卸载，缓存上限为`max(24,可见持有数)`，释放后回落到24条；被淘汰的回复重新进入可见区时补读，不因固定冷缓存上限形成可见区反复淘汰循环。核实成功立即更新共享投影并使压缩资格失效，旧补读不能覆盖新核实结果。不对每条消息无条件拉取完整历史。步骤成功、整轮完成、业务结果正确是不同含义。普通文本无空执行卡；审批/unknown明确显示，原始技术记录移入诊断层.

### 事件与消费者

聊天所有事件携带运行内单调`seq`，新增`model_segment {task_id,assistant_message_id,round}`。分配序号与发送由同一串行边界保证，终态只封口一次。前端拒绝旧序号、重复和错归属事件；缺口通过原运行只读快照补齐，不重新调用模型或工具。终态消息是正文权威。

共享CoreEvent向模块公共AiEventPayload同步model_segment，provider-console使用新1.3.0发布消费该事件；1.2.0及更旧包保持不可变、仅保留完整性，不重新执行旧契约。LAN同步事件类型和当前轮文本重置，但不获得联网工具。累计模型文本/推理输出受64KiB边界；中间说明有界持久化且不包括reasoning，不保证服务商实时usage或金额硬预算。

## 伪代码与事务

```text
CONFIGURE:
  derive account + lease + owned session; reserve account slot        // WR-B01
  acquire shared provider credential mutation lock                    // WR-B02
  stage new opaque scope in cleanup outbox; vault store and readback  // WR-B03
  begin short tx; check account not deleting and old pointer CAS       // WR-B04
  stage old scope; commit new JSON reference and acknowledge new       // WR-B05
  release mutation lock; stale lease never changes active UI           // WR-B06
RUN:
  validate explicit access; search requires configured fixed ref       // WR-B07
  add only authorized tools/schema to existing model loop              // WR-B08
  before tool: validate shape, budget, lease; persist running activity  // WR-B09
  validate HTTPS target, resolve public addresses and pin connection   // WR-B10
  each redirect: validate new target and new DNS snapshot               // WR-B11
  bounded body + inert extraction; create source ID and bounded JSON   // WR-B12
  persist outcome, emit ordered activity, then feed model result       // WR-B13
  model segment: retain bounded prior text; reset current segment      // WR-B14
FINISH:
  seal terminal once; frontend reconcile original run snapshot         // WR-B15
RECOVER:
  read old facts only; settle abandoned read-only web activity failed  // WR-B16
```

没有SQLite事务跨网络/vault await。配置事务包含账户删除检查、旧引用比较、新指针与cleanup ack；外部vault写入不成功则旧指针保留，失败清理通过outbox可重试。Web凭据使用ProviderCredentialScope的独立provider_id和endpoint，不访问模型Provider的secret。`system_settings`只保存JSON编码的不透明UUID引用；账户删除在清理settings前把引用加入provider_credential_cleanup。活跃引用检查包含该指针，避免后台清理删掉正在使用的新密钥。

## 幂等、取消与失败

- 相同聊天请求ID返回原ack，不重复执行；web_access参与请求身份校验，不允许同ID扩大授权。
- 查看/刷新/补读记录均无外部派发。继续是新请求，当前用户重新选择web_access；祖先来源与副作用指纹遵循原有规则。
- WebRuntime在drop中断的dispatch后明确结算pending只读活动；进程崩溃恢复将未结算web步骤标失败，不当作已完成，不制造MCP未知副作用。
- 原MCP dispatch先落账、结果未知阻断、人工报告非权威规则保持。停止不能声称远端操作已回滚。
- 搜索配置读取只说明保存状态；凭据不可读、被撤销或被上游拒绝时显示真实失败。配置保存不等于API认证验证。
- 网页和工具内容不可信，不能修改工具白名单、授权、用户身份、网络边界或调用凭据。

## 分支到验证追踪

| 分支 | 场景 | 目标资产/命令 |
| --- | --- | --- |
| WR-B01～06 | 跨账户、失效租约、配置冲突、vault失败、清理与删除 | `cargo test -p dgos-workbench --lib web::`；`cargo test -p dgos-db user_scope_tests` |
| WR-B07～09 | 默认关闭、选择不继承、未配密钥、次数/上下文上限 | web单元回归、chat_tools与前端执行体验回归 |
| WR-B10～12 | 混合DNS、私网/映射地址、重定向、响应与文本边界、HTML惰性提取 | `cargo test -p dgos-workbench --lib web::` |
| WR-B13～16 | 真实阶段、回执/来源、分段、重复/迟到事件、取消与恢复 | chat_tools/chat_generation、chat store/回复测试、实际界面smoke |
| 完整链路 | 已配置真实Provider + 真实公开HTTPS + 引用 + 相同请求零执行 | `assistant_web_smoke`显式opt-in入口；Brave真实搜索另需配置搜索密钥 |
| 公共消费者 | 新模块轮次不残留旧前言、包完整性、LAN不增权 | provider-console-ai、module-runtime-host及bundled-module pin检查 |

## 发布与回滚

新增能力默认关闭。既有聊天不要求搜索密钥；fetch与search独立。保留旧消息/运行可读，缺少来源/seq的历史快照不能补造事实。没有修改已执行migration；新增配置沿既有账户settings，必要元数据沿现有聊天步骤回执。

provider-console以新不可变版本发布并重新计算pin；旧版退休不篡改历史包。撤销网络能力只阻止新请求，不能撤回已经发送的查询。回退不重放任务；凭据清理outbox仍由宿主处理，不能删除outbox来掩盖清理失败。

## 变更记录

| 日期 | 变更 | 依据 |
| --- | --- | --- |
| 2026-11-11 | 受控联网、回复过程、事件分段和凭据生命周期实施契约 | D147；用户授权全批次实施 |
