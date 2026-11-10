---
title: AI 助手运行基础技术设计
version: V1
technical_task: ASST-FOUNDATION
spec_status: Ready
risk: L3
updated: 2026-11-10
---
# AI 助手运行基础技术设计

用户授权实施 [F1～F6 全批次](../../02-产品与版本/AI助手运行基础完善推进计划.md)，由助手选择以下保守实现。沿用 D136、D140、D141；本技术任务不开放新的场景应用、后台自治、CLI 权限或 LAN 工具权限。实现与证据只记 [V1 实现状态](../../02-产品与版本/当前版本/V1-实现状态.md)。

来源：[FR-006](../../03-功能规格/V1/02-应用模块/06-AI工作台.md) · [公共 AI 服务](D136-AI公共服务技术设计.md) · [接口](V1-接口契约.md) · [数据](V1-数据模型.md)。本文件是本批新增语义的技术依据；正式接口/功能主文档随实现集中同步，不以旧文档缺字段作为绕过宿主验证的理由。

## 风险与对策

| 风险 | 严重程度 | 对策 |
| --- | --- | --- |
| 模块借聊天资产 ID 越权 | 高 | 内容读取绑定宿主 user 与 scope，模块为 module:<id>，聊天为 chat:<session>；不接受前端 scope/user |
| 文本携带权限指令 | 高 | 来源内容保持不可信，固定工具集合与每步权限校验独立于模型文本 |
| 输出结束但交付丢失 | 高 | 运行审计与限期内容分离，明确 opt-in；原 run 读取/确认不重新调用 Provider |
| 继续执行重放副作用 | 高 | 工具派发先落账，确定回执后才能继续；继承稳定参数指纹，不以模型 call ID 作为去重保障 |
| 结果或资源无界增长 | 高 | 单请求文本上限仍24KiB；正文64KiB；资源24小时租期/每scope24MiB；最多保留32份正文，淘汰只删除正文不抹掉运行事实 |
| 控制请求被工作槽占满 | 高 | 宿主只读核实/取消使用独立2个控制槽，普通执行仍8槽；不接受客户端自选命令类别 |
| 模型不遵守结构 | 高 | 声明为生成后校验；外部schema引用拒绝，有限schema/深度，不自动收费修复 |

## 单元契约与保守取舍

- 公共请求新增可选 `foundation` 配置：技能手动/按需、上下文引用、text/json_schema 输出契约、显式结果保留、来源运行、新生成/显式摘要操作。默认保留旧文本行为，不自动载入全部技能、导入资料或生成摘要。
- 资源导入仅 UTF-8 plain/markdown、PNG/JPEG/WebP；文本1MiB、图片8MiB，复用既有验证行为，不执行脚本/宏或远程获取。导入生成随机资源 ID、宿主计算 SHA-256、固定过期时间。引用指定 hash、范围、inline/on_demand；哈希只用于版本绑定，不授予权限。删除/过期/撤权后不继续读取。
- 模块内容域不读取 `chat_assets`；采用独立、受账户/来源限制的临时内容仓储，不建业务作品库。聊天既有资产与消息保留原所有权，新增公共资源也绑定当前聊天会话。
- 同一模块 JS realm 不是文档安全沙箱；scope 的主体是已授权模块。单个运行固定本次引用子集、技能pin、工具schema；文档名称只用于呈现，不能伪称可信身份或跨模块授权。当前runtime只能控制自身运行；重开后只读核实与显式新意图继续。
- 上下文保留宿主硬上限及模型窗口安全预留，供检查和派发共用；实际提供的片段记录 ID/hash/偏移/字节数，不记为“模型已理解”。图片必须满足模型视觉能力声明，且不得通过文本工具返回伪装图片理解。
- 交付回执仅在对应请求跨过Provider传输发送尝试边界后确认；模型预检或adapter本地构造失败不确认，按需读取尚未进入后续模型请求的片段仍为pending。网络尝试不保证服务端接收或模型理解，失败/unknown仍如实保留。
- 公共 executor 接收可信 LocalTools 实现，MCP与宿主资源读取共用一个循环；按需 Skill 沿现有 SkillRuntime。资源/技能读取前后验证租约、授权、固定版本及预算。只读本地集合不获得 MCP 许可。
- 输出 `json_schema` 明确为生成后校验，不声称 Provider 原生结构保证；不支持/超量schema外发前拒绝，生成不符合schema则校验失败。校验成功也不等于业务审批或业务保存成功。
- JSON Schema最大16KiB、深度32、遍历/展开2048节点；仅支持无环本地JSON-pointer引用，外部、递归/动态、anchor引用及未知dialect显式拒绝，不静默降级。
- 结果保留默认为关闭，用户显式开启才持久正文/部分检查点；控制回执和工具指纹仍可记录为无正文运行事实。内容最长24小时、每scope最多32份正文。普通日志/审计不保存正文、reasoning、工具原参数或凭据。过期回读显示 expired/无内容，不解释为从未执行。
- 成功生成后的领取和确认是有界幂等本地操作；不调用模型/工具，不代表应用已经保存成果。确认仅记录领取时间，重读在租期内仍可用。
- 显式继续需要同scope有保留的终态进展、无未决/unknown工具和审计失败；新用户意图形成新运行与来源关联，旧批准无效，已执行指纹继承。不存在无条件自动恢复或自动重派发。
- 模块继续沿原运行及祖先关联核实来源元数据，最多32层、累计128条回执，超限明确拒绝；祖先正文过期不抹掉来源事实，但资源删除/过期仍阻断继续。历史来源核实不混入本轮实际交付片段回执。
- 显式摘要使用当前明确选择的Provider，需确认额外调用；源引用固定且重验，保留原内容。来源运行摘要/新进展是非权威上下文，无法查询的外部结果不总结为成功。
- 运行仍绑定宿主账户和模块attachment；切换标签/隐藏不自动取消，真正卸载/账户结束取消，发送失败保守停止并结算。这里不授予脱离模块生命周期的后台许可。历史/内容快照与原事件seq用于重新观察，不回放旧执行。

## 伪代码

```text
PREPARE:
  bind host account/source/lifecycle; reject forged scope              // AF-B01
  validate capabilities, schema, resource ownership/hash/range          // AF-B02
  prepare fixed skill/resource set and one context budget               // AF-B03
  if continuation: verify retained terminal and effect receipts         // AF-B04
  commit audit + bounded content identity before external dispatch      // AF-B05
RUN:
  before each model/local read/tool revalidate original authorization   // AF-B06
  on text delta accumulate bounded checkpoint; batch persist if opted   // AF-B07
  validate exact tool parameters, consume approval once                 // AF-B08
  commit tool dispatch receipt; network outside transaction             // AF-B09
  commit observed receipt; unknown/audit failure prevents progression   // AF-B10
FINALIZE:
  validate requested output contract; no automatic repair call           // AF-B11
  seal one terminal; save retained content separately from audit         // AF-B12
READ_OR_ACK:
  authorize current source and bounded control admission                 // AF-B13
  return original/expired content or acknowledge without dispatch        // AF-B14
RECOVER:
  mark interrupted; preserve uncertain effects; never auto dispatch      // AF-B15
```

## 分支到验证追踪

| 分支 | 验证场景 | 目标资产 |
| --- | --- | --- |
| AF-B01～03 | 跨账户/模块引用、过期hash、UTF-8边界、预算和非法schema | foundation/core、内容仓储与application行为回归 |
| AF-B04～05 | 继续前unknown阻断、新身份/来源关联、开始失败无外发 | 内容仓储和module/chat运行smoke |
| AF-B06～08 | 撤权、技能pin、参数不完整、过期批准、注入文字 | 共享执行器和已有tool_flow/chat_tools |
| AF-B09～10 | 派发后崩溃、不确定结果、相同参数不同call ID | 持久回执与新进程smoke |
| AF-B11～12 | 非法结构、输出限额、终态/内容保存失败 | 输出校验与application终态回归 |
| AF-B13～14 | 8个执行槽满载仍能核实/取消、重复领取、过期结果 | router饱和与真实宿主接线 |
| AF-B15 | 重启不派发、账户删除/租约失效、原内容可核实 | 内容仓储恢复与双入口smoke |

## 并发、事务与幂等

没有数据库事务跨网络 await。开始、派发、回执与终态使用短事务和running-only CAS；身份/授权必须由宿主固定，异步步骤后重验。内容持久化失败不能把请求标成成功，原审计事实不因正文删除而消失。资源删除与下一次读取有序，已外发数据不能宣称撤回。
- 检查点写入等待期间必须继续驱动执行future，避免同一SQLite连接被步骤准入占用而与检查点相互等待；审计警告在事务前归一为非成功终态，不能留下失去活动执行器的running记录。

新内容表通过追加 migration 实现；账户删除清理所有来源，聊天会话删除清理其scope。模块卸载与撤权不让旧运行复活；只读历史仍遵循已启用可信模块的现行入口。相同 run 的开始/终态不可重置，领取确认幂等；新运行只来自用户明确动作。

## 外部调用与费用

Provider/MCP沿已有超时、逐步准入、预算及不重放语义；本批无新增自动重试或自动选模型。现有额度计算仅对可信次数/Token/字节/时间作上限，未知实际费用不得记0。真实上游验收要求指定端点、模型、工具、可外发资料及额度；缺授权不调用生产凭据，只完成本地机制并保留阻塞。

## 发布与回滚

集中检查、运行smoke、review并修复后交付；不逐次编辑运行全套检查。provider-console按受控新版本发布与pin更新，原历史包只用于已有恢复完整性，不在旧包里偷偷改契约。新契约消费者同批迁移；保留字段缺省用于明确未启用能力，不制造第二执行路径。数据库只前向迁移，代码回退不重做外部副作用。

## 变更记录

| 日期 | 变更 | 依据 |
| --- | --- | --- |
| 2026-11-10 | 全批次实施的保守输入/输出/资源/恢复与控制语义 | 用户授权 ASST-F1～F6；具体实现由助手选定 |
