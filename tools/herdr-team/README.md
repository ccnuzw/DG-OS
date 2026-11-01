# DG-OS herdr 团队配置

工程任务：`ENG-HERDR-TEAM`。这是本地开发协作设施，不新增 DG-OS 产品功能、不改变 V1/V2 的开发或发布门禁。

## 日常入口

在 herdr 选择 `DG-OS Team | 01 指挥`，进入角色 Tab，直接向 `dgos-lead` 提出目标，例如“先为 FR-002 Batch B 的首个真实来源制定最小切片计划，先不编码”。也可以把具体目标写到 `.herdr-team/tasks/goal.md`，再运行 `send lead`。不必分别指挥四个成员；需要改变团队规则时也先告诉 lead。

原有 DG-OS 会话继续独立存在，不自动纳入此团队。若它们与新团队同时改共享文档/编号/契约，必须先由用户和 lead 明确所有权；本团队的单写者规则不会自动约束旧会话。首次正式任务前重新读取当前实现状态，不沿用建立时的进度摘要。

规则权威来源：[智能体团队工作流](../../docs/01-项目概览/智能体团队工作流.md)。`team.json` 声明角色与启动参数；`roles/` 是角色专属入职说明；`task.template.json` 是交接模板。

```sh
node scripts/herdr-team.mjs status
node scripts/herdr-team.mjs doctor
node scripts/herdr-team.mjs brief lead
node scripts/herdr-team.mjs setup
node scripts/herdr-team.mjs start all
node scripts/herdr-team.mjs onboard all
node scripts/herdr-team.mjs check-task .herdr-team/tasks/TASK.json
node scripts/herdr-team.mjs send lead .herdr-team/tasks/goal.md
node --test tools/herdr-team/tests/team.test.mjs
```

`setup/start/onboard/send` 必须在 `HERDR_ENV=1` 的 herdr pane 中执行。首次运行 `setup` 会创建专属控制 workspace，以及四个关联 worktree workspace；每个角色的 agent 与普通检查终端放在不同 Tab，检查终端全宽。保留本机插件自动添加的 reviewr 差异 pane，不关闭未知 pane。所有创建默认 `--no-focus`。`start` 不自动回答信任/认证/权限提示；失败后先运行 `status`，不要重复启动。`onboard` 只下发只读入职，不开业务任务。

本机 ID、基线 SHA、worktree 位置和已尝试入职状态在 `.herdr-team/state.json`，不入 Git。`setup` 在创建前先持久化含唯一标记的操作意图，创建后立即记录返回的 ID。进程在其间中断时拒绝再次创建；先 `doctor` 按唯一标记、branch/path 核对遗留资源，再人工修复登记。不删除、不接管未知 pane，不把别人的 ID 填进去。控制锁带 PID 与随机所有权标记，只有原 PID 确认已退出时才能恢复；不完整或不可验证的锁不自动抢占。

角色 agent 的默认模型由各工具现有配置决定，不猜测模型 ID、不修改全局模型或认证。Codex 开发者采用 workspace-write；Codex 审查者采用 read-only；其余工具保留原有审批机制。角色规则是协作约束，不是完整 OS 沙箱，不能声称它们强制阻止一切越权。

`send` 只向已登记且空闲的准确 agent 提交文件，并且要求它仍在登记的 pane；派发前保存内容 SHA256、尝试 ID 与不确定状态，不保存任务正文。已尝试的相同内容默认拒绝重发，超时也不例外。只有读过目标 UI 并确认需要重发后，才用 `send ROLE FILE --resend-confirmed` 有意重发。`send/onboard` 等待 herdr 观察到工作活动并最多等待 120 秒进入稳定状态。不把送达或稳定等同验收成功；超时不会终止 agent，先读取 UI 再决定如何继续。它不是完整 DAG/预算引擎：任务依赖、资源登记、超时、两轮返工与人工批准由 lead 按工作流执行。

恢复时先 `doctor/status`，核对 Git worktree、pane 的前台进程与认证提示；继续已有会话优先。只有明确退出的空 shell pane 才能 `start ROLE`。不重启 herdr server、不使用强制清理、不关闭已有旧会话。

Codex 首次启动可能打开未信任 hooks 的信息面板；此时表面 idle 不代表文本已进入会话。`onboard` 若 stalled，先读取 UI。可以只关闭信息面板，不选择 trust all、不绕过 hook 信任；确认会话输入为空且原指令未执行后，才通过 `brief` 生成文件并有意重发一次。需要真正批准权限/信任/认证时由用户处理。错误输出不回显完整任务正文。

规则文件在首次建立时尚未提交，初始 worktree 从已有 HEAD 创建；入职 prompt 明确通过控制仓库的绝对路径读取最新团队规则。提交团队配置后，新 worktree 自带规则；更新旧角色时需再次入职，不能误认为 worktree 自动跟随主目录的未提交修改。
