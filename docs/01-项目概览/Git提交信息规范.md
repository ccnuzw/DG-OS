# Git 提交信息规范

本文档定义 DGOS 项目的 Git 提交信息规范，遵循 [Conventional Commits](https://www.conventionalcommits.org/) 标准。

---

## 1. 提交信息格式

### 1.1 基本格式

```
<类型>(<范围>): <简短描述>

[可选的详细描述]

[可选的页脚]
```

**示例**：
```
feat(providers): add retry mechanism for provider calls

Implement exponential backoff retry for 429 and 5xx errors.
Retry up to 3 times with delays of 1s, 2s, 4s.

Closes #123
```

### 1.2 格式说明

| 部分 | 必需 | 说明 |
|------|------|------|
| **类型** | ✅ | 提交类型（feat、fix 等） |
| **范围** | ⚠️ | 影响的模块或功能（可选但推荐） |
| **简短描述** | ✅ | 一句话说明（不超过 72 字符） |
| **详细描述** | ❌ | 详细说明（可选） |
| **页脚** | ❌ | 关联 Issue、破坏性变更（可选） |

---

## 2. 提交类型

### 2.1 标准类型

| 类型 | 说明 | 示例 |
|------|------|------|
| **feat** | 新功能 | `feat(skills): add skill installation UI` |
| **fix** | 修复 Bug | `fix(providers): handle null response from API` |
| **docs** | 文档更新 | `docs: update README with setup instructions` |
| **style** | 代码格式（不影响功能） | `style: format code with rustfmt` |
| **refactor** | 重构（不改变功能） | `refactor(mcp): simplify connection logic` |
| **perf** | 性能优化 | `perf(providers): cache model list for 24h` |
| **test** | 测试相关 | `test(modules): add unit tests for loader` |
| **build** | 构建系统或依赖 | `build: upgrade tauri to 2.1.0` |
| **ci** | CI/CD 配置 | `ci: add rust clippy check` |
| **chore** | 其他杂项 | `chore: update dependencies` |

### 2.2 类型选择指南

**新增功能** → `feat`
- 新增 API 接口
- 新增 UI 组件
- 新增配置项

**修复问题** → `fix`
- 修复崩溃
- 修复逻辑错误
- 修复 UI 显示问题

**代码重构** → `refactor`
- 提取公共函数
- 简化逻辑
- 改善代码结构

**性能优化** → `perf`
- 减少内存占用
- 加快响应速度
- 优化数据库查询

**文档更新** → `docs`
- 更新 README
- 更新功能规格
- 更新注释

---

## 3. 范围（Scope）

### 3.1 后端范围

| 范围 | 说明 | 示例 |
|------|------|------|
| `providers` | Provider 适配（`crates/dgos-providers`） | `feat(providers): add Claude adapter` |
| `skills` | 技能中心（`crates/dgos-skills`、`skills/`） | `fix(skills): reject oversized package` |
| `mcp` | MCP 管理（`crates/dgos-mcp`） | `feat(mcp): add heartbeat detection` |
| `workbench` | 工作台任务编排（`crates/dgos-workbench`） | `fix(workbench): stop replay after cancel` |
| `modules` | 模块系统（`crates/dgos-modules`） | `refactor(modules): split publisher` |
| `host` | Tauri 宿主与 commands（`src-tauri`） | `fix(host): derive caller from window` |
| `db` | 实体与 migration（`crates/dgos-db`） | `feat(db): add providers table migration` |
| `auth` | 用户认证与权限（FR-008/009，规划中） | `feat(auth): add local user sessions` |

### 3.2 前端范围

| 范围 | 说明 | 示例 |
|------|------|------|
| `ui` | 统一设计系统组件（FR-005，规划中） | `feat(ui): add Button component` |
| `workbench` | AI 工作台页面 | `fix(workbench): show unknown tool result` |
| `settings` | 系统设置及 Providers/Skills/MCP 管理页 | `feat(settings): add provider management` |
| `canvas` | 无限画布（V2） | `feat(canvas): add zoom controls` |

### 3.3 通用范围

| 范围 | 说明 | 示例 |
|------|------|------|
| `docs` | 文档 | `docs(docs): update architecture diagram` |
| `build` | 构建配置 | `build(deps): upgrade vite to 8.4` |
| `ci` | CI/CD（尚未配置） | `ci: add rust test workflow` |
| `test` | 测试 | `test(providers): add integration tests` |

---

## 4. 简短描述

### 4.1 编写规则

- ✅ 使用祈使句（"add" 而非 "added"）
- ✅ 首字母小写
- ✅ 不超过 72 字符
- ✅ 不以句号结尾
- ✅ 描述"做什么"而非"怎么做"

### 4.2 好的示例

```
✅ feat(providers): add retry mechanism
✅ fix(skills): handle timeout errors
✅ docs: update installation guide
✅ refactor(mcp): simplify connection logic
✅ perf(db): add index on provider_id
```

### 4.3 不好的示例

```
❌ feat(providers): Added retry mechanism.        # 使用了过去式，有句号
❌ fix(skills): Fix the bug                       # 太模糊
❌ docs: Updated the documentation for installation  # 太长（>72字符）
❌ refactor: Changed some code in MCP              # 不清楚改了什么
❌ I fixed a bug                                   # 没有类型和范围
```

---

## 5. 详细描述

### 5.1 何时需要详细描述

- 复杂的功能实现
- 重要的 Bug 修复
- 破坏性变更
- 需要解释的设计决策

### 5.2 详细描述格式

```
feat(providers): add retry mechanism

Implement exponential backoff retry for provider calls:
- Retry on 429 (rate limit) and 5xx errors
- Max 3 retries with delays: 1s, 2s, 4s
- Do not retry on 4xx errors (except 429)

This improves reliability when provider APIs are temporarily unavailable.

Refs: #123
```

### 5.3 编写要点

- 空一行后开始详细描述
- 说明"为什么"要做这个改动
- 列出关键实现点
- 提及相关的 Issue 或 PR

---

## 6. 页脚（Footer）

### 6.1 关闭 Issue

```
Closes #123
Closes #123, #456
Fixes #789
Resolves #999
```

### 6.2 关联 Issue

```
Refs #123
Related to #456
See also #789
```

### 6.3 破坏性变更

```
BREAKING CHANGE: remove deprecated API endpoint /v1/old

The old endpoint has been removed. Use /v2/new instead.

Migration guide: docs/migrations/v2.md
```

---

## 7. 完整示例

### 7.1 简单功能

```
feat(ui): add dark mode toggle
```

### 7.2 Bug 修复

```
fix(providers): handle null response from OpenAI API

When the API returns null instead of error object,
the app crashes. This fix checks for null before parsing.

Fixes #234
```

### 7.3 重构

```
refactor(modules): extract common loader logic

Move duplicate loader code into shared utility function.
This reduces code duplication and improves maintainability.

No functional changes.
```

### 7.4 破坏性变更

```
feat(auth)!: change JWT token expiry to 24h

BREAKING CHANGE: JWT token expiry changed from 7 days to 24h

Users will need to re-login more frequently for security.
Update your client code to handle token refresh.

Closes #567
```

---

## 8. 分支命名规范

### 8.1 分支类型

| 类型 | 格式 | 示例 |
|------|------|------|
| 功能 | `feature/<name>` | `feature/provider-retry` |
| 修复 | `fix/<name>` | `fix/skill-timeout` |
| 重构 | `refactor/<name>` | `refactor/mcp-connection` |
| 文档 | `docs/<name>` | `docs/update-readme` |
| 测试 | `test/<name>` | `test/provider-integration` |

### 8.2 命名要点

- 使用小写字母和连字符
- 简短且描述性
- 与 Issue 编号关联：`feature/123-provider-retry`

---

## 9. PR 标题规范

PR 标题遵循与提交信息相同的格式：

```
feat(providers): add retry mechanism
fix(skills): handle timeout errors
docs: update installation guide
```

### 9.1 PR 描述模板

```markdown
## 概述
简短描述这个 PR 做了什么

## 变更内容
- 新增了 XXX 功能
- 修复了 XXX 问题
- 重构了 XXX 模块

## 测试
- [ ] 单元测试通过
- [ ] 集成测试通过
- [ ] 手动测试通过

## 关联 Issue
Closes #123

## 截图（如果有 UI 变更）
![screenshot](url)

## 检查清单
- [ ] 代码通过 lint 检查
- [ ] 添加了必要的测试
- [ ] 更新了相关文档
- [ ] 通过了 CI 检查
```

---

## 10. 工具支持

### 10.1 使用 commitizen

安装 commitizen 辅助提交：

```bash
# 全局安装
npm install -g commitizen cz-conventional-changelog

# 在项目根目录初始化
commitizen init cz-conventional-changelog --save-dev --save-exact

# 使用 git cz 代替 git commit
git add .
git cz
```

### 10.2 使用 commitlint

验证提交信息格式：

```bash
# 安装
npm install --save-dev @commitlint/{config-conventional,cli}

# 配置
echo "module.exports = {extends: ['@commitlint/config-conventional']}" > commitlint.config.js

# 添加 Git hook
npx husky add .husky/commit-msg 'npx --no -- commitlint --edit "$1"'
```

### 10.3 VS Code 扩展

推荐扩展：
- **Conventional Commits**: 自动补全提交类型
- **GitLens**: 查看提交历史

---

## 11. 常见错误

### 11.1 错误示例与修正

| 错误 | 问题 | 正确 |
|------|------|------|
| `Added new feature` | 缺少类型和范围 | `feat(providers): add retry mechanism` |
| `fix: bug fix` | 描述太模糊 | `fix(skills): handle timeout errors` |
| `feat(providers): Added retry.` | 过去式 + 句号 | `feat(providers): add retry mechanism` |
| `Update docs` | 缺少类型 | `docs: update installation guide` |
| `fixed the thing` | 不规范 | `fix(module): describe what was fixed` |

---

## 12. 检查清单

提交前检查：

- [ ] 提交信息包含类型
- [ ] 简短描述清晰且不超过 72 字符
- [ ] 使用祈使句，首字母小写
- [ ] 如果需要，添加了详细描述
- [ ] 关联了相关 Issue
- [ ] 破坏性变更标注了 BREAKING CHANGE

---

## 13. 参考资源

- [Conventional Commits 官方规范](https://www.conventionalcommits.org/)
- [Angular 提交信息规范](https://github.com/angular/angular/blob/main/CONTRIBUTING.md#commit)
- [commitlint 文档](https://commitlint.js.org/)

---

**最后更新**：2026-10-28  
**维护人**：开发团队  
**适用范围**：所有 DGOS 仓库
