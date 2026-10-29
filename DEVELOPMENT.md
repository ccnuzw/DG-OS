# DG-OS Development Guide

DG-OS 是本地优先的 AI 应用开发框架，基于 Tauri 2 + Rust + React + TypeScript 构建。

本文是开发速查。环境要求、目录结构、可选测试和常见问题以 [工程与本地环境](docs/01-项目概览/工程与本地环境.md) 为准；工程约束见 [AGENTS.md](AGENTS.md)；功能进度只看 [V1 实现状态](docs/02-产品与版本/当前版本/V1-实现状态.md)。

## 技术栈

| 层 | 当前采用 |
|----|---------|
| 桌面 | Tauri 2 |
| 后端 | Rust workspace（`src-tauri` + `crates/dgos-*`）、SeaORM 1.1 + SQLite、tokio、reqwest |
| 前端 | React 19、TypeScript 6、Vite 8、Zustand 4、React Router 6 |
| 测试 | Rust 内置测试（mockito、tempfile）、Vitest 5 + Testing Library + jsdom |

局域网 HTTP 服务（Axum）、Module Federation 和统一组件库都还在规划中，代码里没有引入。

## 常用命令

```bash
npm install            # 安装依赖
npm run tauri:dev      # 开发模式（Vite 1420 端口 + 桌面窗口）
npm run tauri:build    # 构建桌面安装包
npm test               # 前端测试
npm run build          # 技能包校验 + design:check + tsc + vite build
npm run design:check   # 内联样式、硬编码颜色、未定义变量；存量清单只降不升（D068）
npm run design:prune   # 迁移页面后收缩存量清单（只能减少，存在违规时拒绝）
npm run contracts:check
cargo check --offline --workspace --all-targets
cargo test --offline --workspace
```

## 如何添加 Tauri 命令

命令按领域放在 `src-tauri/src/commands/` 下（`provider_commands.rs`、`skills.rs`、`workbench.rs`、`mcp.rs`），由 `commands/mod.rs` 统一导出，在 `src-tauri/src/lib.rs` 的 `generate_handler!` 中注册。

每个命令都必须接收 `WebviewWindow`，并先通过 `host::context` 取得宿主派生的调用上下文。用户身份、模块身份和调用来源不能作为参数传入（D023），`npm run contracts:check` 会检查这一点：

```rust
// src-tauri/src/commands/<domain>.rs
#[tauri::command]
pub async fn my_command(
    window: WebviewWindow,
    input: MyInput,
    state: State<'_, MyState>,
) -> Result<MyOutput, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::ProvidersRead)
        .map_err(|e| e.to_string())?;
    state.do_work(context.user_id(), input).await.map_err(|e| e.to_string())
}
```

```rust
// src-tauri/src/lib.rs
.invoke_handler(tauri::generate_handler![
    // ...
    commands::my_command,
])
```

新增或修改命令属于接口变更，需要同步 [V1 接口契约](docs/04-技术架构/当前版本/V1-接口契约.md)。

## 前端调用命令

```typescript
import { invoke } from "@tauri-apps/api/core";

const result = await invoke<MyOutput>("my_command", { input });
```

## 文档

- [文档中心](docs/README.md)
- [V1 版本总览](docs/02-产品与版本/当前版本/V1-版本总览.md)
- [V1 总体架构](docs/04-技术架构/当前版本/V1-总体架构.md)
- [V1 数据模型](docs/04-技术架构/当前版本/V1-数据模型.md)
- [V1 接口契约](docs/04-技术架构/当前版本/V1-接口契约.md)
- [Git 提交信息规范](docs/01-项目概览/Git提交信息规范.md)

## 许可证

尚未添加 LICENSE 文件（产品蓝图计划采用 MIT）。
