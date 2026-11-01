# DGOS - 本地优先的 AI 应用开发框架

**DG-OS**（DGOS，Distributed General Operating System for AI）是基于 Rust + Tauri 2 的本地优先 AI 应用开发框架。V1 目标是打通 Provider 适配、技能中心、MCP 管理、模块系统和统一设计系统的底层能力闭环；无限画布在 V2 交付。

> ⚠️ **当前状态**：V1 开发中，暂未发布。各功能实现进度只在 [V1 实现状态](docs/02-产品与版本/当前版本/V1-实现状态.md) 维护。

---

## ✨ 核心特性

下表区分已有实现切片和 V1 目标；“已实现”指代码中已有可运行切片，不代表功能已验收。

| 能力 | 说明 | 当前情况 |
| --- | --- | --- |
| 🔌 Provider 适配 | 统一接口调用 OpenAI 兼容与 Anthropic 协议模型，凭据存入系统钥匙串 | 已实现核心切片 |
| 🛠️ 技能中心 | 随应用发布的官方 `SKILL.md` 内容包，工作台按固定版本使用 | 官方本地库已实现；第三方来源未开始 |
| 🧰 MCP 管理 | stdio / Streamable HTTP 连接、工具发现、逐次确认调用 | 已实现快速迭代版 |
| 🤖 AI 工作台 | 多轮聊天（流式生成、历史、Markdown），可选技能与 MCP 工具 | 本地验证（D080）；真实上游只对一个第三方网关验证过 |
| 🔥 模块热更新 | 受信任的本地模块包动态加载与热更新 | 本地验证（macOS，D056）；跨平台、签名与公证延后到发布阶段 |
| 🎨 统一设计系统 | 所有模块复用统一主题与组件 | 开发中；VoiceOver 人工清单未执行 |
| 👥 本地多用户 | 本地账户登录、切换与按用户数据隔离 | 本地验证（D087、D090） |
| 🌐 局域网协同 | 局域网内浏览器访问主机 | 规划中；没有 HTTP 服务代码，设置页只有“尚未启用”占位 |
| 📦 强制模块化 | 按领域拆分 crate，限制单文件行数 | 持续约束 |

---

## 🚀 快速开始

### 环境要求

- **Rust**：1.90+（当前 Tauri 2.12 依赖要求）
- **Node.js**：22.22.2+、24.15.0+ 或 26+（jsdom 30 的引擎要求）
- **npm**：随 Node.js 安装
- **操作系统**：macOS（V1 首发平台）；Windows 在 V1 发布门禁后补齐，Linux 计划 V2

### 安装与启动

```bash
git clone https://github.com/ccnuzw/DG-OS.git
cd DG-OS

npm install
npm run tauri:dev
```

`npm run tauri:dev` 会启动 Vite 开发服务器（端口 1420）并打开桌面窗口。SQLite 数据库 `dgos.db` 位于系统的应用数据目录，启动时自动执行迁移。

### 常用命令

```bash
npm test                         # 前端测试（Vitest）
npm run build                    # 依赖版本检查 + 技能包校验 + 样式检查（design:check）+ TypeScript + Vite 构建
npm run design:check             # 样式合规（ESLint + Stylelint + 存量清单，D068）
npm run contracts:check          # 桌面 IPC 宿主契约静态检查
cargo test --offline --workspace # Rust 测试
```

在 Apple Silicon 上运行 Rust 测试如遇链接错误，见 [工程与本地环境](docs/01-项目概览/工程与本地环境.md) 中的 arm64 环境变量说明。

### 首次使用

1. 打开 Providers 页面，添加一个 OpenAI 兼容 Provider 并测试连接
2. 在 Skills 页面浏览官方技能，按需启用
3. 打开 Workbench，选择模型（可选一个技能）提交任务

---

## 📚 文档

| 文档 | 说明 |
|------|------|
| [文档中心](docs/README.md) | 统一文档入口 |
| [V1 版本总览](docs/02-产品与版本/当前版本/V1-版本总览.md) | V1 功能范围 |
| [V1 实现状态](docs/02-产品与版本/当前版本/V1-实现状态.md) | 当前进度与验证证据（唯一来源） |
| [功能规格](docs/03-功能规格/V1/README.md) | 10 个 V1 功能规格 |
| [技术架构](docs/04-技术架构/README.md) | 总体架构和技术设计 |
| [冻结决策](docs/06-决策记录/V1-冻结决策.md) | 跨功能产品与工程决策 |

---

## 🏗️ 技术栈

| 层次 | 当前采用 | 规划（尚未引入） |
|------|---------|----------------|
| **桌面** | Tauri 2 | - |
| **后端** | Rust workspace（`src-tauri` + `crates/dgos-*`）、SeaORM 1.1、tokio、reqwest | 局域网 HTTP 服务（FR-010） |
| **前端** | React 19、TypeScript 6、Vite 8、Zustand、React Router 6 | - |
| **数据库** | SQLite（单文件 `dgos.db`） | - |
| **测试** | Rust 内置测试、Vitest 5 + Testing Library、macOS 真实 WebView 冒烟（`npm run smoke:tauri*`） | E2E 框架 |
| **模块系统** | 本地模块包发布器与页面加载（`dgos-modules`、`src/modules/`，Blob URL 动态 `import()`，D042） | Module Federation 加载 adapter（未引入） |
| **设计系统** | Shadcn/ui 组件源码 + Radix + Tailwind 4（`src/ui/`，别名 `@dgos/ui`，D066） | - |

详见 [技术栈选型](docs/04-技术架构/技术栈选型.md)。

---

## 🎯 版本路线图

| 版本 | 主题 | 状态 |
| --- | --- | --- |
| V1 | 底层能力框架 + AI 工作台、系统设置 | 开发中 |
| V2 | 无限画布 | 规划中 |
| V3 | 应用市场与云端账号 | 规划中 |
| V4 | 云端同步与实时协同 | 规划中 |
| V5 | 企业化 | 规划中 |
| V6 | 移动端 | 规划中 |
| V7 | AI Agent | 规划中 |
| V8 | 开发者生态 | 规划中 |

版本只使用 V1–V8 整数编号，详见 [版本路线图](docs/02-产品与版本/版本路线图.md)。

---

## 🤝 贡献指南

### 提交代码

1. Fork 本仓库
2. 创建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'feat: add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

### 提交信息规范

遵循 [Conventional Commits](docs/01-项目概览/Git提交信息规范.md)：

```
feat: 新功能
fix: 修复 bug
docs: 文档更新
style: 代码格式（不影响功能）
refactor: 重构
test: 测试相关
chore: 构建/工具链相关
```

工程约束见 [AGENTS.md](AGENTS.md)。

---

## 📖 从老项目学习

DGOS 由 infinite-canvas-ccnuzw 项目重构而来：

- 老项目：Python + FastAPI，`main.py` 单文件 19,038 行，无法维护
- 新架构：Rust 按领域拆分 crate，限制单文件行数
- 老项目：无文档、无测试、无规范
- 新架构：规格驱动开发，功能、决策、实现状态分开维护

老项目问题的原始记录见 [初始讨论记录](docs/90-参考资料/2024-01-XX-初始讨论完整记录.md) 第一部分。

---

## 🔒 许可证

计划采用 MIT（见[产品蓝图](docs/02-产品与版本/产品蓝图.md)第 8.1 节），仓库尚未添加 LICENSE 文件。

---

## 📧 联系方式

- **项目主页**：https://github.com/ccnuzw/DG-OS
- **问题反馈**：https://github.com/ccnuzw/DG-OS/issues

---

## 🌟 致谢

- [Tauri](https://tauri.app/) - 轻量级桌面应用框架
- [Model Context Protocol](https://modelcontextprotocol.io/) - 工具协议
- [SeaORM](https://www.sea-ql.org/SeaORM/) - Rust ORM
