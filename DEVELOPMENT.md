# DG-OS Development Guide

## 项目初始化成功 ✅

DG-OS 是一个本地优先的 AI 应用开发框架，基于 Tauri 2 + Rust + React + TypeScript 构建。

## 技术栈

### 后端
- **Tauri 2**: 桌面容器
- **Rust 1.75+**: 后端语言
- **Axum 0.7**: Web 框架
- **SeaORM 0.12**: 数据库 ORM
- **SQLite 3.45+**: 本地数据库

### 前端
- **React 18**: UI 框架
- **TypeScript 5.0+**: 类型安全
- **Vite 5.0+**: 构建工具
- **Zustand 4.0+**: 状态管理
- **React Router 6**: 路由管理

## 项目结构

```
DG-OS/
├── src/                    # 前端源码
│   ├── components/         # React 组件
│   ├── pages/             # 页面组件
│   ├── stores/            # Zustand 状态管理
│   ├── utils/             # 工具函数
│   ├── types/             # TypeScript 类型定义
│   ├── App.tsx            # 主应用组件
│   └── main.tsx           # 入口文件
├── src-tauri/             # Rust 后端源码
│   ├── src/
│   │   ├── commands.rs    # Tauri 命令
│   │   ├── lib.rs         # 库入口
│   │   └── main.rs        # 主程序
│   ├── Cargo.toml         # Rust 依赖配置
│   └── tauri.conf.json    # Tauri 配置
├── docs/                  # 项目文档
├── public/                # 静态资源
├── package.json           # npm 配置
└── vite.config.ts         # Vite 配置
```

## 开发命令

### 安装依赖
```bash
npm install
```

### 开发模式（热重载）
```bash
npm run tauri:dev
```

这会同时启动：
- Vite 开发服务器（前端，带 HMR）
- Tauri 应用窗口（后端 Rust）

### 构建生产版本
```bash
npm run tauri:build
```

### 仅构建前端
```bash
npm run build
```

### 仅检查 Rust 编译
```bash
cd src-tauri && cargo check
```

## 开发流程

### Week 1-2: 基础框架层（当前阶段）
✅ **已完成**:
- [x] Tauri 应用骨架
- [x] Rust 后端骨架（Axum）
- [x] React 前端骨架（Vite）
- [x] 基础路由系统

⏳ **待实现**:
- [ ] SQLite 数据库初始化
- [ ] 本地用户管理
- [ ] 用户登录界面

### Week 3-4: Provider 适配框架
- [ ] Provider trait 定义
- [ ] OpenAI 适配器实现
- [ ] 模型列表获取
- [ ] 模型调用接口

详见：`docs/02-产品与版本/当前版本/V1-实现状态.md`

## 验证项目

### 1. 检查 Rust 编译
```bash
cd src-tauri && cargo check
# 应该输出: Finished `dev` profile [unoptimized + debuginfo] target(s)
```

### 2. 检查前端构建
```bash
npm run build
# 应该输出: ✓ built in XXXms
```

### 3. 启动开发模式
```bash
npm run tauri:dev
# 应该打开 DG-OS 应用窗口，显示欢迎界面
```

## 文档

完整的架构设计和 API 文档请查看：
- [V1 版本总览](docs/02-产品与版本/当前版本/V1-版本总览.md)
- [V1 总体架构](docs/04-技术架构/当前版本/V1-总体架构.md)
- [V1 数据模型](docs/04-技术架构/当前版本/V1-数据模型.md)
- [V1 接口契约](docs/04-技术架构/当前版本/V1-接口契约.md)

## 常见问题

### Q: 如何添加新的 Tauri 命令？
A: 在 `src-tauri/src/commands.rs` 中定义命令，然后在 `lib.rs` 中注册：
```rust
// commands.rs
#[tauri::command]
pub fn my_command(arg: String) -> String {
    format!("Received: {}", arg)
}

// lib.rs
.invoke_handler(tauri::generate_handler![
    commands::greet,
    commands::my_command,  // 添加这里
])
```

### Q: 如何在前端调用 Rust 命令？
A:
```typescript
import { invoke } from "@tauri-apps/api/core";

const result = await invoke<string>("my_command", { arg: "hello" });
```

### Q: 开发模式下端口被占用怎么办？
A: 修改 `vite.config.ts` 中的 `server.port` 配置。

## 下一步

1. 实现 SQLite 数据库初始化（参考 V1-数据模型.md）
2. 实现用户登录界面和认证逻辑
3. 开始 Provider 适配框架开发

## 贡献

详见：`docs/01-项目概览/Git提交信息规范.md`

## 许可证

TBD
