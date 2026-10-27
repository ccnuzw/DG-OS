# V1 Skills 内容包与发布规范

| 属性 | 值 |
| --- | --- |
| 日期 / 范围 | 2026-10-27 / V1-FR-002 官方内容包技术基线及后续发布设计 |
| 需求依据 | [D016](../../06-决策记录/V1-冻结决策.md#d016-skills-定位为官方优先的本地技能中心)、[V1-FR-002](../../03-功能规格/V1/01-底层能力/02-Skills管理系统.md) |
| 实现与验收状态 | 只在 [V1 实现状态](../../02-产品与版本/当前版本/V1-实现状态.md) 维护；本文区分当前技术契约与后续设计，不等于完整发布验收 |

## 1. 技术决策

- Skill 是 `SKILL.md` 指令和受限文本资源，不是 HTTP/CLI 程序。下载和读取不授予权限，不安装运行时、不执行附带脚本。
- 原生 Agent Skills 正文保持可移植；`skill.json` 是 DGOS 官方发布侧车元数据，不要求第三方上游原本具备它。第三方适配器后续负责规范化，不能因此虚构来源或版本。
- 官方包、目录、用户状态分离；内容按 `(skill_id, version)` 不可变，同版本不同哈希拒绝登记，更新必须换版本。
- 保留旧表、旧记录和文件；旧包不自动转换、不继续下载安装或执行。旧粗粒度调用参数不用于新技能执行。
- 当前只支持随应用发布的官方目录，第三方和独立远程更新明确关闭；没有远程 registry 也能启动和浏览。

## 2. 三层目录

### 2.1 内容源码（唯一维护位置）

```text
skills/
  categories.json                  # 六个 DGOS 一级分类
  catalog.v1.json                  # 发布目录及每包完整内容 SHA-256
  official/<category>/<slug>/
    skill.json
    SKILL.md
    LICENSE
    references/                    # 可选，仅文本
    templates/                     # 可选，仅文本
    examples/                      # 可选，仅文本
scripts/skills-catalog.mjs         # 发布检查/更新清单
```

测试资产在 `crates/dgos-skills/tests/` 和 `tests/`，不作为内容文件分发。一级分类固定为 `writing`、`visual`、`video-audio`、`documents-office`、`data-research`、`development-automation`；每技能一个主分类、多标签，分类与可用应用是不同维度。

### 2.2 应用发布资源

Tauri 将 `../skills/` 映射到应用 `resource_dir/skills/`，不维护第二份源码副本。开发模式从 `CARGO_MANIFEST_DIR/../skills` 读取；正式构建仅从应用资源读取，缺失或损坏不能静默伪装为有效空库。

首版使用解压目录，不强制创建每技能 ZIP。`catalog.v1.json` 的原始字节通过 Rust `include_bytes!` 固定在二进制中；运行时目录字节必须与该发布副本一致，随后逐包核验哈希。信任来自应用发布链，不来自包自报的 `identity` 或“verified”。这不是远程数字签名验证。

### 2.3 本地不可变快照

```text
<app-data>/skills/
  bundled/<skill-id>/<version>/    # 经校验的独立快照，不直接使用可变源码目录
  staging/bundled-<operation-id>/  # 校验、复制后原子 rename；失败清理
  packages/                       # 保留未来下载包用途，当前不开放
```

数据库版本记录指向快照。应用升级保留旧版本文件，已登记用户继续引用原版本；启用/禁用不回写文件。用户不能卸载内置资源，只能禁用。快照被篡改时拒绝加载，不覆盖篡改文件或删旧包。孤立完整快照可在下次初始化校验后重新登记；完整事务恢复、遗留 staging 清理和任务引用回收仍需下一阶段设计。

## 3. `skill.json` 与 `SKILL.md`

```json
{
  "schema_version": "1.0",
  "id": "dgos.official.writing.summarize",
  "slug": "summarize",
  "name": "内容总结",
  "description": "将长文本整理为有依据的结构化摘要",
  "version": "1.0.0",
  "identity": "dgos_official",
  "category": "writing",
  "tags": ["summary", "document"],
  "entry": "SKILL.md",
  "locales": ["zh-CN"],
  "compatible_apps": ["ai-workbench"],
  "required_tools": [],
  "author": {"name": "DGOS Team", "type": "organization"},
  "license": {"spdx": "Apache-2.0", "redistributable": true},
  "source": {"kind": "dgos_official", "path": "official/writing/summarize", "revision": "content-release-1.0.0"},
  "verification": {"status": "not_evaluated", "scope": []}
}
```

- `schema_version` 必须是 `1.0`，`entry` 必须为 `SKILL.md`，版本为真实 SemVer。未知顶层字段拒绝解析，不能夹带 shell、启动命令或授权声明。
- 官方 ID 采用 `dgos.official.<category>.<slug>`。包不能仅凭该前缀取得官方身份；必须来自应用固定目录。
- `SKILL.md` 使用 `name`/`description` frontmatter 与正文，发布时名称对应 slug。DGOS 机器显示名称、分类和版本以侧车元数据为准，不要求正文包含非标准的版本字段。
- `LICENSE` 为完整许可文本，不是单行许可名称。官方精选未来需保留原作者、上游、修订和许可；未明确再分发许可的内容不能内置。
- `verification` 是质量/兼容性证据，不是哈希通过与否。首包不虚构工作台验证，统一 `not_evaluated`；通过完整性校验也不得显示“安全无风险”。
- `required_tools` 只描述依赖，不授予权限；首包为空。`compatible_apps` 只声明目标场景，不表示相应业务应用已经实现。

## 4. 目录与哈希契约

```json
{
  "schema_version": "1.0",
  "catalog_version": "1.0.0",
  "skills": [{
    "id": "dgos.official.writing.summarize",
    "version": "1.0.0",
    "path": "official/writing/summarize",
    "sha256": "64位小写十六进制"
  }]
}
```

列表名称、许可和兼容场景从经包哈希保护的 `skill.json` 读取，不在目录重复维护。目录最大 128 项，不接受重复 ID。

哈希算法：以 UTF-8 相对路径排序，对每个普通文件累加 `path bytes + NUL + file bytes + NUL`，最终 SHA-256（小写十六进制）。覆盖 `skill.json`、正文、完整许可及全部文本资源。每文件 SHA-256 另存数据库 `file_manifest` 用于追溯。

当前硬限制：每包最多 128 个文件、文件与目录合计最多 256 项、路径深度最多 8 层、单文件 256 KiB、总计 2 MiB；仅 UTF-8 文本，资源路径限 ASCII 字母、数字及 `/._-`。顶层仅允许 `skill.json`、`SKILL.md`、`LICENSE`；辅助目录仅允许 `.md`、`.txt`、`.json`、`.csv`。拒绝路径穿越、绝对路径、符号链接、特殊文件、`scripts/`、可执行包和超限资源。第三方安装仍关闭，不能把这些本地限制当成压缩包解压安全已实现。

维护命令：

```bash
npm run skills:catalog  # 修改内容/正式发布时先提升版本，再重新生成目录哈希
npm run skills:check    # 不写文件，验证格式、分类、路径、资源和哈希
npm run build          # 构建前强制执行 skills:check
```

构建时哈希检查不替代人工许可/内容质量审阅或真实模型验证。前端错误、日志和元数据不得存储凭据。

## 5. 数据模型与迁移

物理权威：`crates/dgos-db/src/migrations/m004_create_skill_content_tables.rs`，为旧数据库增量创建三表，不修改既有三张 Skills 表。

| 表 | 主键/唯一约束 | 职责 |
| --- | --- | --- |
| `skill_catalog` | `skill_id` | 稳定身份、名称、最新版本、分类/标签、作者/许可、来源、兼容应用及元数据 |
| `skill_versions` | `id=skill_id@version`；unique(skill_id, version) | 内容哈希、快照路径、内置标记、每文件摘要、版本元数据与登记时间 |
| `skill_user_states` | 复合主键(user_id, skill_id)，FK(version_id) | 用户选定版本、启用状态和创建时间；不与共享内容混存 |

初始化：验证所有发布包 → staging 复制/重新校验 → rename 到快照 → 数据库事务登记目录和版本。失败不登记半可用行。重复初始化不重复版本，不改变用户禁用状态；重复“添加”幂等且不重启用已禁用项。旧版本不可覆盖，应用升级不能自动改写用户版本选择。

旧 `skills`、`skill_registry_cache` 和 `skill_usage_logs` 保留。列表 DTO 暂时兼容旧桌面调用方：新 Skill 的 `call_type=content` 与合成 manifest 只作传输适配，**不写回旧 `skills` 表**。新内容永不进入 HTTP/CLI 执行分支。旧启用项允许禁用，不允许重新启用/更新/删除/执行，防止共享旧路径或未审核命令造成破坏。未来迁移需独立清单和用户确认。

## 6. 桌面能力接口

本轮仅桌面 Tauri Commands，不声称已同步 HTTP OpenAPI 或开放局域网管理。

| Command | 当前行为 |
| --- | --- |
| `fetch_skills_registry` | 兼容命名；立即读取官方目录与已有 Legacy cache，无公网请求；本地来源为 `bundled://official` |
| `search_skills` | 查询本地已登记目录和 Legacy cache；分类与来源筛选由界面执行 |
| `get_skill_detail` | 官方内容返回元数据、完整性哈希、正文、许可、来源修订和质量验证状态；每次读取前验证快照 |
| `install_skill` | 官方包只创建用户状态（界面“Add to My Skills”），幂等；拒绝 Legacy 下载 |
| `list_installed_skills` | 当前用户状态 + 旧格式记录；内容快照不随用户重复复制 |
| `toggle_skill` | 按用户切换官方内容启用；Legacy 只许禁用 |
| `prepare_content_skill` | 要求该用户已添加/启用、适用工作台且无未解决工具依赖；返回 `skill_id, version, content_hash, body`，不是任务成功结果 |
| `call_skill` | 内容型不得执行；旧 HTTP/CLI 也返回迁移提示，不再启动进程 |
| 旧更新/卸载/配置 Commands | 保留调用身份并拒绝操作；完整更新与用户副本另行实现 |

桌面错误沿用当前 `Result<T, String>`，不宣称已发布新的结构化错误码协议：缺少用户/版本返回 `SkillNotFound`，禁用返回 `SkillDisabled`，不适用场景或未解决工具返回 `DependencyNotSatisfied`，内容完整性/非法路径/同版本冲突返回 `InvalidManifest`，Legacy 操作返回 `LegacyFormatUnsupported`。读取或初始化 I/O/数据库失败原样转为错误，不返回空正文或成功占位；未来远程接口需要单独冻结错误契约。

详情接口预览目录最新版本；已添加用户的内容准备仍读取所选版本。界面明确标注最新内容预览，不在预览或重复添加时自动更新用户版本。

`user_id` 沿用当前桌面参数（页面仍是本地 `default-user`）。数据库按用户隔离不等于已建立可信身份认证；这些参数不可作为远程鉴权证据，不得直接暴露给局域网用户。接入用户会话前，所有写操作和读取需从可信宿主会话派生用户身份。

任务上下文预算、冲突处理、绑定表、活动任务引用、工具逐次授权、敏感日志脱敏及真实模型结果仍属于后续工作台集成，不通过一个“prepare”接口冒充完成。

## 7. 首批内容与交付边界

| 分类 | slug | 用途 |
| --- | --- | --- |
| writing | summarize | 有据摘要，区分事实与推断 |
| writing | rewrite | 保持原意的文本改写 |
| documents-office | report-outline | 基于材料的报告提纲 |
| documents-office | meeting-actions | 决策与行动项 |
| data-research | source-synthesis | 来源归纳与证据缺口 |
| development-automation | code-review | 静态风险审阅，不声称执行代码 |

清单不是完整技能生态；图像、视频等分类预留，不展示虚构技能。6 个包仅表示内容已提供和完整性可校验，真实模型质量门禁未验收。用户点击“添加”不代表工作台已经能自动使用。

## 8. 验证与后续阶段

- `crates/dgos-skills/tests/bundled_catalog.rs`：全新离线初始化、目录搜索/详情、重复初始化、禁用保留、用户状态隔离、旧数据增量迁移、非法目录/资源/哈希、篡改后读取拒绝，关联 AC16/18/19/23/24 的部分边界。
- `tests/skills-center.test.tsx`：官方身份/许可/正文、来源筛选、添加、禁用及错误交互；不把 mock 视为真实桌面/模型验收。
- 后续：真实工作台任务与固定版本引用（AC21/35）→ 第三方适配/固定修订安全安装（AC25–31）→ 用户确认更新、任务占用、用户副本（AC32–34）。
- `.dskill.zip`、独立发布目录签名、公钥轮换、远端认证与限流属于后续分发协议；当前客户端不接受这些远程包。实现解压和验证前不得开放安装。
- 版本状态与实际命令证据只写入实现状态/验证记录，不以本规范存在代替测试通过。
