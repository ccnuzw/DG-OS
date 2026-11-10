---
title: AI 助手持续完善技术设计
version: V1
technical_task: CHAT-CONTINUOUS
spec_status: Ready
risk: L3
updated: 2026-11-09
---
# AI 助手持续完善技术设计

来源：[持续完善方案](../../02-产品与版本/AI会话持续完善方案草案.md) · [AI 助手规格](../../03-功能规格/V1/02-应用模块/06-AI工作台.md) · [公共执行核心](D136-AI公共服务技术设计.md)。用户于2026-11-09授权实施全部批次；保守取舍由助手选定。本文定义独立技术任务 CHAT-CONTINUOUS，不提升 V2/V7 门禁。规范性增量落入功能主文档，接口与物理结构分别以接口契约和新增 migration 为准，状态只回写 V1 实现状态。

## 风险与对策

| 风险 | 严重程度 | 对策 | 关联依据 |
| --- | --- | --- | --- |
| 取消时按钮切换导致再次发送 | 高 | 非提交停止事件、请求身份及终态保护，生产交互验证 | CHAT-P1 |
| 派发后失败被误报为未执行 | 高 | 保留派发、unknown、usage、审计结果，不自动重试 | CHAT-P1 |
| 摘要丢失约束或覆盖新历史 | 高 | 显式调用、原文保留、来源范围与摘要提交CAS | CHAT-P2 |
| 技能内容或资源越权 | 高 | 授权集合、固定包哈希、路径/大小校验、不执行脚本 | CHAT-P3 |
| 流式参数未完成便执行工具 | 高 | 完整协议结束与schema验证后才能审批，批准后重新准入 | CHAT-P4 |
| 恢复重放外部副作用 | 高 | 只读检查点、未知暂停、用户显式新运行，不恢复旧批准 | CHAT-P5 |

## 单元契约

| 单元 | 前置条件 | 返回／后置条件 | 副作用 | 幂等 |
| --- | --- | --- | --- | --- |
| 上下文检查 | 当前账户、目标会话及资产归属有效 | 分项预算、淘汰与拒绝原因；估算与真实usage区分 | 无模型调用、不写消息 | 只读 |
| 显式整理 | 用户明确同意外发与费用；原生成槽空闲 | 真实模型摘要、来源范围与hash；失败保留原文 | 账户chat域持久摘要与独立运行审计 | 固定客户端请求身份，无自动重派发 |
| 按需技能消费 | 用户选择固定可用集合并开启按需模式 | 目录、正文或资源及精确内容无关回执 | 仅受控包读取、使用审计 | 固定pin及资源身份，不替换版本 |
| 流式工具步骤 | 原账户租约、固定Provider、选择schema有效 | 增量文字、真实阶段、完整提案或失败 | 批准后才调用MCP | 原审批只能消费一次 |
| 运行核实 | 当前账户拥有原会话 | 已持久步骤/部分正文/结果状态 | 无外部调用，不恢复执行 | 只读 |
| 显式继续 | 原检查点可核实、无待核实副作用、当前配置有效 | 新请求身份与来源运行关联 | 仅执行新动作 | 新请求按原发送幂等语义，旧动作不重放 |
| 人工核实报告 | 原工具步骤未知、用户已在应用外核实并显式确认 | 保留原unknown回执，另记用户报告/说明；不冒充宿主确认 | 仅账户chat存储；不调用外部工具 | 重复报告不重放动作，稳定副作用摘要仍阻止重复 |

## 伪代码与分支

```text
PREPARE(input, host):
  capture host owner + irreversible lease                          // CHAT-B01
  validate session/assets/provider/pins and original request id     // CHAT-B02
  if replay: return original receipt; conflict: reject              // CHAT-B03
  compute one shared context budget; if impossible reject          // CHAT-B04
  reserve per-owner generation slot; persist start transaction     // CHAT-B05
RUN:
  before each dispatch recheck lease/config/pins                     // CHAT-B06
  stream bounded text/reasoning/tool parameter buffers             // CHAT-B07
  if tool args incomplete/invalid: never expose approval           // CHAT-B08
  after complete proposal await exact user decision                // CHAT-B09
  approve: recheck scope/schema and consume one approval            // CHAT-B10
  dispatch MCP; persist known outcome or unknown, never replay      // CHAT-B11
  if unknown/audit failure: stop progression                        // CHAT-B12
  seal terminal and commit message/run evidence transaction         // CHAT-B13
SUMMARY:
  capture source range/hash and destination; explicit model call    // CHAT-B14
  compare source before commit; stale/cancel/failure: retain old     // CHAT-B15
  persist summary in chat content domain, never delete original     // CHAT-B16
CONTINUE:
  read owned checkpoint; if unresolved side effect: block           // CHAT-B17
  validate current lease/config and create new explicit request     // CHAT-B18
  pass confirmed progress as untrusted context; no old approvals    // CHAT-B19
```

## 分支到验证追踪

本任务使用独立技术分支标识，不占用既有 B06 编号。新增功能验收与正式 AC 同步由功能主文档维护；下表是测试责任，不是通过声明。

| 分支 | 场景 | 目标验证资产 |
| --- | --- | --- |
| CHAT-B01～03 | 账户结束、重复发送、冲突身份 | chat_generation 与宿主聊天命令回归 |
| CHAT-B04～05 | 附件/模型窗口预算、并发准入及开始事务 | 上下文与聊天持久化回归 |
| CHAT-B06～08 | 逐步撤权、增量协议损坏、参数未闭合 | Provider协议及chat工具回归 |
| CHAT-B09～12 | 拒绝/过期/schema变化、MCP unknown与审计失败 | tool_flow、chat_tools及真实宿主smoke |
| CHAT-B13 | 取消/终态竞态、审计与正文一致 | chat_generation、chat_repository |
| CHAT-B14～16 | 摘要真实输出、失败保留、来源变更、隔离 | 新摘要行为回归和本机Provider回环smoke |
| CHAT-B17～19 | 崩溃、只读核实、显式继续、不重放 | 文件SQLite重开与聊天继续执行smoke |

## 并发控制

保留每账户一个活动生成槽，摘要与继续使用同一槽。账户代次不可复活。无数据库事务跨网络await；短期锁只保护准入和原子审批消费。摘要来源使用范围/哈希CAS，失败不覆盖已有有效内容。流式检查点有界批量落盘，不逐token写入。共享执行核心只有一个所有者协调变更，不建立第二套状态机。

## 事务边界

开始：任务、用户消息、助手占位与能力引用同事务。终态：消息状态与任务状态一致提交，checkpoint引用同源。摘要提交核对来源后写账户chat域；运行审计不写正文。新增schema只追加migration，保持已有迁移原文。账户与会话删除同步清理新内容，失败回滚，不留下跨账户可读数据。

## 幂等与重试

沿用客户端UUID和持久开始回执，重复请求只核实。未明确证明未派发的请求不自动补发。摘要、继续、工具回合没有新的自动重试循环；底层Provider现有有界协议重试仍按其准入/派发语义。恢复不承诺上游恰好一次；无法查询外部结果时保留unknown并阻断自动继续。

恢复判断区分Provider计费/输出未知与工具副作用未知。仅前者允许用户以新意图显式继续已持久进展，历史消息保持unknown；未解决的工具派发/unknown步骤及审计异常仍阻断。人工报告不改写历史成功状态，不提供自动重试许可；新运行继承已完成副作用摘要，不能用新模型call ID绕过不重放。

## 外部调用与补偿

摘要使用用户选择的当前Provider，先展示费用与数据外发确认。模型/工具步骤沿已有超时、调用次数和输出上限；不存在自动扩大预算。MCP无通用回滚能力，不以取消承诺撤销。技能读取不调用网络安装，不执行脚本。真实服务验收需隔离账户和明确端点/额度授权，本地替身只证明本地链路。

## 安全与日志

用户指导、摘要、技能和工具返回均为不可信上下文，不能提升权限。正文、密钥、工具参数/结果不进入一般日志；确需持久的内容归账户chat域并适用会话删除。技能读取限固定包内允许资源，禁止路径穿越、符号链接绕过及跨包读取。模型参数流完整且验证通过后才出现审批，参数片段不能触发外部行为。

## 发布与回滚

前置：受影响消费者、宿主契约、迁移、前端交互及本机运行smoke集中通过。新能力可由用户不启用，但不得用空实现或假成功兜底。数据库版本不向旧结构破坏性回退；停止运行后保留原消息和检查点供只读核实。外部副作用不通过代码回退重做。性能与成本先记录真实基线，不伪造阈值达标。

## 变更记录

| 日期 | 变更 | 依据 |
| --- | --- | --- |
| 2026-11-09 | 全批次实施的保守契约、并发、持久化及不重放边界 | 用户明确授权 CHAT-P1～P6 |
