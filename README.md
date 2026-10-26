# DGOS - 本地优先的 AI 应用开发框架

**DG-OS**（Developer Groundwork Operating System）是一个基于 Rust + Tauri 的本地优先 AI 应用开发框架，提供 Provider 适配、Skills/MCP 管理、模块热更新和无限画布等核心能力。

> ⚠️ **当前状态**：V1.0 开发中，暂未发布

---

## ✨ 核心特性

- 🚀 **本地优先**：所有数据存储在本地 SQLite，云端服务可选
- 🔌 **Provider 适配**：统一接口调用 OpenAI、Claude、Gemini 等 AI 模型
- 🛠️ **Skills/MCP 管理**：使用标准协议扩展 AI 能力
- 🔥 **模块热更新**：Module Federation 实现无需重启的模块加载
- 🎨 **统一设计系统**：所有模块使用统一 UI 组件库，强制视觉一致性
- 🌐 **局域网协同**：HTTP 服务自动启动，远程设备可访问
- 📦 **强制模块化**：Rust 编译期保证，避免单文件巨石代码

---

## 🚀 快速开始

### 环境要求

- **Rust**: 1.75+
- **Node.js**: 18+
- **Python**: 3.10+ (可选，用于 Skills)
- **操作系统**: macOS (V1.0), Windows/Linux (V1.1+)

### 安装

```bash
# 克隆仓库
git clone https://github.com/your-org/DG-OS.git
cd DG-OS

# 安装依赖
cd server && cargo build
cd ../frontend && pnpm install

# 启动开发环境
pnpm dev
```

### 首次使用

1. 启动应用后创建本地用户
2. 配置 Provider（OpenAI 兼容）
3. 打开 AI 工作台，开始对话

详见 [工程与本地环境](docs/01-项目概览/工程与本地环境.md)

---

## 📚 文档

| 文档 | 说明 |
|------|------|
| [文档中心](docs/README.md) | 统一文档入口 |
| [V1 版本总览](docs/02-产品与版本/当前版本/V1-版本总览.md) | V1.0 功能范围 |
| [V1 产品需求](docs/02-产品与版本/当前版本/V1-产品需求.md) | V1.0 详细需求 |
| [功能规格](docs/03-功能规格/V1/README.md) | 10 个核心功能规格 |
| [技术架构](docs/04-技术架构/README.md) | 总体架构和技术设计 |
| [冻结决策](docs/06-决策记录/V1-冻结决策.md) | D001-D013 关键决策 |

---

## 🏗️ 技术栈

| 层次 | 技术 |
|------|------|
| **后端** | Rust + Axum + SeaORM |
| **桌面** | Tauri 2 |
| **前端** | React + TypeScript + Vite |
| **数据库** | SQLite |
| **模块系统** | Module Federation |
| **设计系统** | @dgos/ui (自研) |

详见 [技术栈选型](docs/04-技术架构/技术栈选型.md)

---

## 🎯 版本路线图

### V1.0 基础框架（开发中）

- ✅ Provider 适配框架
- ✅ Skills/MCP 管理系统
- ✅ 模块系统与热更新
- ✅ 统一设计系统
- ✅ AI 工作台
- ✅ 系统设置
- ✅ 本地用户管理
- ✅ 模块权限管理
- ✅ 局域网协同

**目标**：打通底层能力框架最小闭环，验证架构可扩展性

### V1.1-V1.5 画布系统（规划中）

- V1.1: Windows 支持 + 更多 Provider 适配器
- V1.2: 画布核心（视口、缩放、拖拽）
- V1.3: 10+ 种节点类型
- V1.4: 实时执行、循环节点
- V1.5: 工作流导入导出、应用市场

### V2.0+ 云端与协同（未来）

- 云端账号与同步
- 实时协同编辑
- 移动端支持
- AI Agent 能力
- 开发者生态

详见 [版本路线图](docs/02-产品与版本/版本路线图.md)

---

## 🤝 贡献指南

我们欢迎各种形式的贡献！

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

---

## 📖 从老项目学习

DGOS 是从 infinite-canvas-ccnuzw 项目重构而来，吸取了以下教训：

- ❌ **老项目**：Python + FastAPI，单文件 19,038 行，无法维护
- ✅ **新架构**：Rust 强制模块化，每个文件 < 1000 行
- ❌ **老项目**：Electron 过重（>100MB）
- ✅ **新架构**：Tauri 轻量（<10MB）
- ❌ **老项目**：无文档、无测试、无规范
- ✅ **新架构**：spec-docs-pro 规格驱动开发

详见 [老项目教训分析](docs/90-参考资料/老项目教训分析.md)

---

## 🔒 许可证

MIT License - 详见 [LICENSE](LICENSE) 文件

---

## 📧 联系方式

- **项目主页**: https://github.com/your-org/DG-OS
- **文档**: https://docs.dg-os.dev
- **问题反馈**: https://github.com/your-org/DG-OS/issues

---

## 🌟 致谢

感谢以下开源项目：

- [Tauri](https://tauri.app/) - 轻量级桌面应用框架
- [LangChain](https://www.langchain.com/) - Skills 标准协议
- [Anthropic MCP](https://modelcontextprotocol.io/) - Model Context Protocol
- [Module Federation](https://module-federation.io/) - 微前端架构

---

**开始使用 DGOS，构建你的 AI 应用！** 🚀
