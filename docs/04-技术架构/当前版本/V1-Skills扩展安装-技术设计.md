---
title: "V1 Skills 扩展安装技术设计"
version: V1
feature_id: V1-FR-002
updated: 2026-11-01
---
# V1 Skills 扩展安装技术设计

来源：[FR-002 主规格](../../03-功能规格/V1/01-底层能力/02-Skills管理系统.md) · [D115](../../06-决策记录/V1-冻结决策.md#d115-fr-002-batch-b-改为离线优先的本地包与受控远程安装) · [内容包与发布规范](V1-Skills内容包与发布规范.md) · 实现状态见 [V1 实现状态](../../02-产品与版本/当前版本/V1-实现状态.md)

本文是 FR-002 Batch B 的独立技术设计。它冻结实现边界、恢复协议和安全默认值，但不替代接口契约、OpenAPI 或 migration；具体命令字段、数字错误码和物理 migration 在实现前分别写回各自权威文档。Batch A 的官方内容、工作台上下文和旧 Legacy 数据边界继续以现有文档为准。

## 1. 设计目标与不包含

### 1.1 目标

- 在无网络时导入、预览和管理本地 `.zip` Skill 包。
- 通过不依赖 Git 可执行文件的 GitHub HTTPS 适配器获取固定修订，保存原始来源、请求 ref、解析出的 commit、内容哈希、许可状态和验证范围。
- 解析受控的 `npx skills add` 输入，让用户可以粘贴常用安装指令；解析器不启动 `npx`、Shell、npm、Git 或包内命令。
- 以“预览 → 用户确认 → 暂存校验 → 原子发布”的过程安装，支持崩溃恢复、重复请求幂等和旧版本回退。
- 显式检查更新；分支或标签不自动升级，在用任务继续引用原版本。

### 1.2 不包含

- 在线 Registry、全站搜索、账号登录、GitHub 私有仓库认证或用户自定义远程域名。
- 执行 Skill 正文、脚本、Shell、npm 生命周期、二进制或自动安装 Python/Node.js。
- 让 `-a codex` 变成权限授予；它只记录兼容目标。
- 把本地路径、访问令牌、Cookie、完整命令行或包正文写入数据库日志。

## 2. 风险与对策

| 风险 | 严重程度 | 对策 | 关联决策 |
| --- | --- | --- | --- |
| 恶意压缩包、路径穿越、符号链接和解压炸弹 | 高 | 先读中央目录，限制压缩/展开大小、文件数、深度和 UTF-8 文本；拒绝链接、特殊文件和可执行内容 | D115、内容包规范 §4 |
| 远程 ref 漂移或内容被替换 | 高 | branch/tag/URL 先解析为 commit；保存 commit 与规范化内容哈希；更新必须显式确认 | D115 |
| 文件系统与 SQLite 提交不一致 | 高 | 持久安装操作日志、独立 staging、rename 后短事务登记、启动恢复；读侧忽略未提交操作 | FR-002 AC30/31 |
| 重试或并发安装重复发布 | 高 | `(用户, 来源身份, 内容哈希)` 锁、幂等键、版本唯一约束和可重放状态机 | FR-002 AC25/29/31 |
| 第三方内容扩大宿主权限或泄露凭据 | 高 | Skill 仅是内容；工具依赖只作声明；每次 Workbench 任务重新执行宿主权限检查；日志只留摘要 | D016、D018 |
| GitHub 限流或不可用阻塞本地使用 | 中 | 外部请求隔离、超时、一次有界重试；缓存只用于预览/读取，不伪造远程在线状态 | D115 |

## 3. 包格式和来源规范

### 3.1 本地 `.zip`

归一化后包根目录必须包含 `SKILL.md`；`skill.json`、`LICENSE` 可选。允许的根文件和目录沿用内容包规范：文本文件扩展名为 `.md`、`.txt`、`.json`、`.csv`，不允许 `scripts/`、可执行文件、特殊文件或符号链接。为兼容常见压缩方式，若全部条目位于同一个顶层目录，可在校验前剥离这一层；混合多个顶层目录时拒绝，不猜测 Skill 根目录。

现有内容包限制继续适用于 Batch B：最多 128 个文件、总条目最多 256、路径深度最多 8、单文件最多 256 KiB、展开总量最多 2 MiB、资源路径仅 ASCII `A-Za-z0-9/._-`。外层 zip 压缩数据最多 8 MiB；中央目录声明的展开总量、压缩比或条目数超过限制时，在写入磁盘前拒绝。加密 zip、重复归一化路径、绝对路径、`..`、NUL 和数据描述符异常均拒绝。

`SKILL.md` 必须是 UTF-8 文本，frontmatter 至少含 `name` 与 `description`，单字段长度分别不超过 128 和 4,096 个 Unicode 标量值。存在 `skill.json` 时只接受 schema `1.0`，未知顶层字段、入口不是 `SKILL.md` 或身份声明与内容不一致均拒绝。没有 `skill.json` 时生成导入身份：`imported.<sha256 前 16 位>`；这只是稳定内容身份，不声称作者或语义版本。没有 `LICENSE` 时允许用户安装，但预览标记 `license_status=missing`，不能进入官方随包发布。

规范化内容哈希沿用官方包算法：按相对路径字节序排序，累加 `path + NUL + file + NUL` 后 SHA-256。每个文件的摘要写入版本元数据，用于篡改检测和恢复，不写正文。

### 3.2 GitHub 来源

V1 只接受以下 HTTPS 形式，主机严格为 `github.com`：

```text
https://github.com/<owner>/<repo>
https://github.com/<owner>/<repo>/tree/<ref>/<skill-subpath>
<owner>/<repo>  [-s|--skill <skill-subpath>]
```

`owner`、`repo`、`ref` 和子路径只允许 GitHub 标识字符，长度分别不超过 100、100、128 和 256；拒绝 query、fragment、userinfo、URL 编码后的路径分隔符、非 HTTPS 和额外路径。V1 的 `ref` 使用单段 tag、branch 或完整 40 位 commit；含斜杠的 branch 需在后续版本扩展，不通过模糊拆分路径规避规则。仓库根目录只有一个 `SKILL.md` 时可省略子路径；发现多个候选时必须由用户指定 `-s`。

适配器用 GitHub REST API 解析 ref 的实际 commit，再从 `codeload.github.com` 下载该 commit 的 zip 归档，在内存/受限临时文件中定位子路径并复用 §3.1 的完整校验。适配器不调用 Git，不接受 GitHub token；私有仓库返回“需要认证”而不是降级匿名。重定向最多一次且目标仍须是 GitHub 允许的两个 HTTPS 主机。

版本标签不是内容身份：无语义版本的 branch/tag 以 `revision:<commit>` 显示；安装记录同时保存请求 ref 和实际 commit。分支只用于后续显式检查更新，安装后不后台轮询、不自动替换用户版本。

### 3.3 `npx skills add` 受控解析

兼容层接收一行文本，长度最多 4 KiB，只支持以下语法：

```text
npx skills add <github-url> -a codex
npx skills add <owner>/<repo> -a codex -s <skill-subpath>
```

`-a codex` 必须恰好出现一次；`-s`/`--skill` 最多一次。tokenizer 只处理空白和成对单/双引号，拒绝反斜杠转义之外的 shell 元字符、命令替换、重定向、环境变量、未知选项、额外位置参数和任何 `--yes`、`--token`、`--exec`。解析结果只产生结构化 `GithubSource`，不会保留原始整行。错误在预览前返回，且不访问网络。

## 4. 架构与模块边界

```text
Skills 页面 / Tauri command
          |
     SourceParser
      /       \
 LocalZip   GithubAdapter ---- reqwest (HTTPS)
      \       /
       PackageValidator
             |
       InstallCoordinator
       /       |        \
   staging  SQLite    Workbench
```

| 模块 | 职责 | 明确禁止 |
| --- | --- | --- |
| `source_parser` | 解析 GitHub URL、`owner/repo` 和受控命令行；规范化来源身份 | shell、`npx`、Git、任意域名 |
| `local_zip` | 文件选择、中央目录检查、受限解压、UTF-8 与 Skill 元数据验证 | 直接覆盖已发布目录 |
| `github` | ref→commit、归档获取、超时/限流/取消、来源缓存 | 保存 token、自动轮询、把 URL 当版本 |
| `package_validator` | 路径、文件、许可、哈希、依赖与权限声明校验 | 运行正文或脚本 |
| `install_coordinator` | 预览、确认、操作日志、并发锁、发布和恢复 | 跨网络请求持有数据库事务 |
| `provenance` | 生成稳定来源身份、版本标签、许可与验证范围 | 把来源声称的 verified 当成信任根 |

所有命令从宿主派生当前用户，不接受 `user_id`、module 身份或能力声明。Settings 页面可调用安装/更新命令；Workbench 只读消费已提交版本，不能安装或改变来源。

## 5. 数据与迁移边界

### 5.1 已有表的增量语义

不修改已执行 migration。`skill_versions.metadata` 扩展为可向后兼容的 `provenance` 对象，至少包含 `source_kind`、`normalized_source`、`requested_ref`、`resolved_commit`（本地包为 null）、`license_status`、`verification_scope`、`file_manifest_version` 和 `version_label_kind`。`skill_catalog.source_kind/source_ref` 仅保存可展示的非敏感来源，不能写绝对本地路径。旧行缺少这些字段时按 `bundled` 或 `legacy` 推导。

### 5.2 安装操作日志（新增 migration，编号在实现时按序分配）

需要新增 `skill_install_operations`，否则无法在进程崩溃后区分“已 rename 但尚未登记”和“用户取消的 staging”。物理列以实现 migration 为准，逻辑字段如下：

| 字段 | 约束与用途 |
| --- | --- |
| `operation_id` | TEXT 主键，宿主随机生成 |
| `user_id` | TEXT 非空，按宿主用户过滤 |
| `idempotency_key` | TEXT 唯一；由用户、规范化来源、内容哈希和操作类型组成，不含正文 |
| `source_kind` / `source_ref` | `local_zip` 或 `github`；来源引用经过规范化 |
| `state` | `previewed`、`publishing`、`committed`、`failed`、`abandoned` |
| `skill_id` / `version_id` / `content_hash` | 校验后的稳定身份和版本引用，可空至预览完成 |
| `staging_relpath` / `published_relpath` | 只存应用数据目录相对路径，禁止绝对路径 |
| `metadata` | 许可、commit、文件清单、验证范围；不得含正文或凭据 |
| `last_error_key` / `created_at` / `updated_at` | 脱敏错误键和生命周期时间 |

操作记录属于 Skills 领域，账户删除时清理用户行；已提交 `skill_versions`、`skill_catalog` 是共享历史，不因某用户删除而清理。孤立目录由恢复器按哈希重新登记或隔离删除，不直接删除在用版本。

## 6. 单元契约与命令边界

### 6.1 单元契约

| 函数 | 前置条件 | 返回/后置条件 | 副作用 | 幂等 |
| --- | --- | --- | --- | --- |
| `parse_source(input)` | 输入 ≤4 KiB | 结构化本地或 GitHub 来源；不访问网络 | 无 | 是 |
| `preview_source(user, source)` | 用户由宿主派生；来源合法 | 返回 operation、内容摘要、许可、来源修订和限制；状态 `previewed` | 受限读取/网络、写 staging 与操作日志 | 同一 idempotency key 返回同一预览 |
| `confirm_install(user, operation)` | operation 属于用户、未过期、摘要未变 | 发布不可变版本并设置用户状态；状态 `committed` | 文件 rename、短事务写版本/目录/用户状态 | 是；重复返回已提交版本 |
| `check_update(user, skill)` | 已有 GitHub provenance | 返回新 commit 的预览或明确无更新 | 有界网络请求，可写缓存 | 是 |
| `recover_operations()` | 应用启动或显式修复 | 所有操作进入终态或可重试状态 | 清理 staging、补登记或隔离 | 可重复 |

### 6.2 计划中的 Tauri Commands

以下是实现前要写回 [V1 接口契约](V1-接口契约.md) 的命令职责，不是当前已注册接口：

| Command | 输入要点 | 语义 |
| --- | --- | --- |
| `parse_skill_source` | `input` | 纯解析受控 URL/命令；不联网 |
| `preview_skill_install` | `source` 或用户选择的本地文件引用 | 创建/复用预览操作，返回摘要和风险提示 |
| `confirm_skill_install` | `operation_id` | 重新校验摘要并安装；身份由宿主派生 |
| `cancel_skill_install` | `operation_id` | 取消并清理未发布操作 |
| `check_skill_updates` | 可选 skill 列表 | 只检查已安装 GitHub 来源，不自动安装 |
| `preview_skill_update` / `confirm_skill_update` | operation、确认 | 展示差异和权限变化后显式发布 |

具体字段、分页、数值错误码和能力声明不得在本设计之外提前复制；旧 `install_skill` 只继续支持官方“添加到我的技能”，不能被重载为任意远程安装。

## 7. 伪代码与分支

### 7.1 预览

```text
FUNCTION preview_source(user, input) -> Preview:
  source = parse_source(input)                                      // B02-27
  IF source is invalid or unsupported: RETURN Err(source_input)    // B02-28
  IF source.kind == local_zip:
    bytes = read_selected_file_with_limits(source.path)             // B02-29
    archive = inspect_zip_central_directory(bytes)                   // B02-30
  ELSE:
    ref = normalize_github_ref(source)                              // B02-32
    commit = resolve_ref_outside_transaction(ref)                   // B02-33
    bytes = fetch_github_archive_with_limits(commit)                // B02-34
    archive = inspect_zip_central_directory(bytes)                   // B02-30
  package = validate_skill_package(archive)                          // B02-31
  identity = canonical_identity(source, package)                     // B02-36
  lock identity for preview deduplication
  existing = find_operation_by_idempotency(user, identity)
  IF existing is active: RETURN existing                             // B02-35
  staging = create_private_staging(user, operation_id)
  write_and_rehash(package, staging)
  IF operation_expired_or_cancelled: cleanup_staging; RETURN Err(preview_expired) // B02-38
  BEGIN TRANSACTION
    INSERT operation(state=previewed, metadata, hashes, staging)
  COMMIT
  RETURN preview_without_body_or_absolute_paths
```

### 7.2 确认、发布和恢复

```text
FUNCTION confirm_install(user, operation_id) -> Installed:
  op = lock_operation_for_user(user, operation_id)                  // B02-39
  IF op.user_id != host_user(user): RETURN Err(permission_denied)    // B02-49
  IF op expired/cancelled OR preview_fingerprint_changed:
    RETURN Err(preview_expired)
  acquire_source_lock(op.identity)                                  // B02-40
  IF committed version already exists: RETURN it                    // B02-41
  verify_staging_hash_and_limits(op.staging_relpath)                 // B02-37
  target = immutable_path(op.skill_id, op.version_id)
  BEGIN TRANSACTION
    UPDATE operation SET state=publishing, published_relpath=target
  COMMIT
  rename_without_overwrite(op.staging_relpath, target)              // B02-42
  BEGIN TRANSACTION
    INSERT skill_catalog/skill_versions if absent
    UPSERT skill_user_states without re-enabling a disabled user
    UPDATE operation SET state=committed
  COMMIT
  RETURN installed

ON startup recover_operations():
  FOR op in non-terminal operations:
    IF final path exists and hash matches:
      finish_metadata_commit(op)                                    // B02-43
    ELSE IF staging exists and op not expired:
      leave retryable and revalidate before next confirmation
    ELSE:
      mark abandoned and remove staging
  remove expired staging and orphan files only when no version/reference uses them // B02-48
```

### 7.3 更新、能力限制和审计

```text
FUNCTION check_update(user, installed):
  IF adapter lacks capability: RETURN Err(capability_not_supported) // B02-45
  resolve current ref and compare commit/hash outside transaction
  IF unchanged: RETURN NoUpdate
  preview new package and compute permission/dependency diff
  IF required_tools or host capabilities grow: require fresh confirmation // B02-43
  preserve old version until new commit succeeds

FUNCTION uninstall_or_disable(user, skill, version):
  IF version is bundled or has active task references:
    return Err(version_in_use) and keep the snapshot                     // B02-44
  remove only the user's state; garbage-collect an unreferenced snapshot later

FUNCTION audit_failure(error, context):
  record only error_key, source_kind, normalized host, operation_id prefix and timing // B02-46
  never record token, Cookie, absolute path, full input, body, prompt or response

ON shutdown:
  cancel network futures, stop new previews, and wait at most 5 seconds   // B02-47
  leave resumable operations for startup recovery; never auto-replay install
```

## 8. 并发、事务、幂等与外部调用

### 8.1 并发控制

| 场景 | 冲突点 | 控制方式 | 失败策略 |
| --- | --- | --- | --- |
| 同一来源并发预览 | staging 与网络 | 规范化身份锁 + idempotency key | 返回已有 operation |
| 同一版本并发发布 | version 唯一键 | source lock + SQLite unique | 读取已提交版本 |
| 更新与 Workbench 任务并发 | 旧版本文件 | 版本不可变；任务持有引用 | 更新只影响新任务 |
| 账户删除与安装 | user 状态 | 宿主账户操作取得领域写锁 | 安装返回账户状态错误，可重试 |

死锁预防：先取得用户/来源锁，再取得 operation 行锁；网络请求永不在 SQLite 事务内；所有锁都有取消路径。

### 8.2 事务边界

- 网络获取、zip 解析、哈希和 staging 写入在事务外。
- `previewed → publishing` 是短事务；文件 rename 不在事务内。
- rename 后的元数据登记与 `skill_user_states` upsert 在同一短事务；失败由操作日志恢复，不把半成品暴露给列表。
- SQLite 默认隔离级别保持当前连接配置；不得以数据库事务宣称文件系统原子性。

### 8.3 幂等与恢复

幂等键为 `SHA-256(user_id || source_identity || content_hash || operation_kind)`，只保存摘要。预览默认 15 分钟有效；过期操作不能确认，staging 最迟在下次启动清理。应用重启不重放远程请求、不自动升级 branch；用户再次确认时可从仍完整的 staging 重试。

### 8.4 GitHub 外部调用

单次 ref 解析和归档下载各 15 秒，整次预览最多 30 秒；响应体和重定向受 §3.2 限制。429 使用服务端 `Retry-After`，最多等待一次且总时限不变；401/403、404、非法 JSON 和不可达不重试；一次 5xx/网络失败重试，使用 250ms 上限退避。取消立即停止请求并清理 staging。缓存按规范化来源+commit+内容哈希隔离，缓存命中仍要重新做包哈希校验。

## 9. 安全、许可与日志

- 本地文件路径只在选取和读取期间使用；数据库仅存相对快照路径和非敏感来源摘要。
- `skill.json` 的 `required_tools`、`compatible_apps` 和许可声明是内容元数据，不授予权限；WorkBench 仍按 D018 在任务和每次工具入口重新校验。
- GitHub 许可状态分为 `present`、`declared_only`、`missing`、`unknown`；未知或限制再分发只阻止官方打包，不阻止用户主动获取，但必须在确认页显示警告。
- 日志只记录错误键、来源类型、提交短标识、内容哈希前缀、耗时和操作 ID 前缀。禁止记录访问令牌、Cookie、User-Agent、绝对路径、完整命令、包正文、Prompt 或回复。
- 预览正文可在内存短暂返回给当前用户，但不得写操作日志；前端卸载或取消后立即释放。

## 10. 分支到测试追踪

| 分支 | AC 映射 | 目标测试资产 |
| --- | --- | --- |
| B02-27～B02-28 | AC26、AC27 | `crates/dgos-skills/tests/source_parser.rs` |
| B02-29～B02-31 | AC26、AC29、AC30 | `crates/dgos-skills/tests/package_validation.rs` |
| B02-32～B02-35 | AC26、AC27、AC28 | `crates/dgos-skills/src/github.rs` 的本地 HTTP fixture 测试 |
| B02-36～B02-37 | AC25、AC29、AC30 | `crates/dgos-skills/src/install.rs`（定向测试待补） |
| B02-37、B02-39～B02-42 | AC29、AC30、AC31 | `crates/dgos-skills/src/install.rs`（定向测试待补） |
| B02-38、B02-43、B02-48 | AC28、AC31 | `crates/dgos-skills/src/install.rs`（故障注入待补） |
| B02-43 | AC32、AC34 | `crates/dgos-skills/tests/package_update.rs` |
| B02-44 | AC33、AC34 | `crates/dgos-skills/tests/package_remove.rs` |
| B02-45 | AC27、AC28 | `crates/dgos-skills/tests/source_capabilities.rs` |
| B02-46 | AC28、AC35 | `crates/dgos-skills/tests/provenance.rs` |
| B02-47～B02-48 | AC31 | `crates/dgos-skills/tests/package_recovery.rs` |
| B02-40 | AC25、AC31 | `crates/dgos-skills/src/install.rs`（来源锁已实现，竞态测试待补） |
| B02-49 | AC24、AC31 | `crates/dgos-db/src/user_scope_tests.rs`、安装命令宿主身份（已覆盖基础清理） |

前端目标资产为 `tests/skills-source-install.test.tsx`（上传、命令预览、确认、失败重试和更新 diff）及 `tests/skills-source-failure.test.tsx`。真实 GitHub 测试必须由 `DGOS_SKILLS_LIVE=1` 显式开启，并固定 commit；默认 CI 使用本地 HTTP fixture，不把网络可用性伪装成本地通过。

## 11. 实施顺序、回滚与发布门禁

1. 先实现纯解析器、zip 校验器和 provenance DTO；为每个限制建立失败测试。
2. 实现 GitHub adapter（ref→commit、归档下载、缓存、超时和取消），仅用 fixture 验证，不开放 UI。
3. 新增 migration、操作日志和恢复器；完成故障注入、并发和账户删除测试。（首个实现切片已完成，测试与恢复边界仍需补齐。）
4. 暴露 Tauri preview/confirm/cancel 命令，加入 Settings 页面 ZIP 上传、来源预览与受控命令输入；更新接口契约和 Host contract 检查。更新命令仍待实现。
5. 接入 Workbench 真实任务并记录固定版本，完成本地包和固定 GitHub commit 的真实闭环。
6. 发布前运行 `skills:check`、Rust/前端回归、真实 GitHub pinned smoke 和打包资源比对；跨平台、签名与服务条款仍属发布阶段。

回滚只关闭新安装入口并保留已提交旧版本；不删除旧快照、不回退已执行 migration。操作日志和 orphan staging 可由恢复器清理，不能用手工删除数据库行代替恢复。

## 12. 变更记录

| 日期 | 变更 | 关联决策 |
| --- | --- | --- |
| 2026-11-01 | 新建 Batch B 技术设计：本地 zip、受控 GitHub、`npx skills add` 解析、安装操作日志、原子发布与恢复、更新和测试门禁 | D115 |
| 2026-11-02 | 实现来源解析、受限 ZIP 校验、外部 manifest/provenance 摘要和固定 commit 的 GitHub HTTPS 适配器；安装操作日志、发布与命令仍待实现 | D115 |
| 2026-11-02 | 实现安装操作 migration/repository、用户隔离 staging、哈希校验、原子发布/启动恢复、五条 Settings 命令（含 ZIP 字节上传）和页面预览确认；更新/卸载、完整测试资产和 Workbench 闭环仍待实现 | D115 |
