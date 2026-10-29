---
title: "Provider 适配框架"
version: V1
feature_id: V1-FR-001
domain: 底层能力
spec_status: Ready
risk: 标准
track: Provider 能力
updated: 2026-10-29
---
# V1-FR-001：Provider 适配框架

> D023 桌面补充基线（2026-10-28）：宿主派生用户身份，Provider DTO 不携带 secret，凭据保存到 OS vault，历史明文字段通过可重试迁移清空。当前字段与迁移规则见 [接口契约 §4.0](../../../04-技术架构/当前版本/V1-接口契约.md#40-当前桌面实施契约d023)、[数据模型 §4.2](../../../04-技术架构/当前版本/V1-数据模型.md#42-provider-管理)；本文早期 AES/SQLCipher 表草图不再作为当前凭据实施依据。前置工作包与证据见 [FR-004 前置收口设计](../../../04-技术架构/当前版本/V1-FR004前置收口设计.md)，不改变既有 AC 编号。§6 中带 `user_id` 的命令草案及 `stream_provider` 均为早期草案；当前已注册命令见 `src-tauri/src/lib.rs`、`src-tauri/src/commands/provider_commands.rs` 和接口契约 §4.0。

## 1. 功能元信息

| 项目 | 内容 |
| --- | --- |
| 编号 | V1-FR-001 |
| 功能名称 | Provider 适配框架 |
| 所属版本 | V1 |
| 领域 | 底层能力 |
| 规格状态 | Ready |
| 优先级 | P0（必须） |
| 风险等级 | 中等（涉及并发调用、缓存管理） |
| 上游依赖 | 无（基础能力） |
| 下游影响 | V1-FR-006（AI 工作台）、V2 画布系统 |
| 规格负责人 | TBD |
| 实现状态 | 见 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)，本文不记录 |

## 2. 功能概述

### 2.1 一句话描述

统一适配各厂商 AI 模型 API，提供模型列表获取、分类标签、同步/流式调用能力，支持多 Provider 配置、排序和手动选择。

### 2.2 详细说明

Provider 适配框架是 DGOS 的底层核心能力，负责屏蔽各个 AI 模型提供商（OpenAI、Claude、通义千问等）的 API 差异，为上层应用提供统一的调用接口。

**核心价值**：
- ✅ 统一接口：应用层不需要关心具体 Provider 的 API 细节
- ✅ 多 Provider 支持：用户可以配置多个同类型 Provider（如多个 OpenAI 账号）
- ✅ 模型分类：自动给模型打标签（text-generation、image-generation 等）
- ✅ 灵活选择：应用层可以手动选择 Provider，或使用默认排序

**用户价值**：
- 开发者：只需要调用统一接口，不用处理各厂商 API 差异
- 最终用户：可以配置多个账号，灵活切换和管理

### 2.3 功能边界

**包含**：
- ✅ Provider CRUD 管理（增删改查）
- ✅ 统一的 Provider trait 接口定义
- ✅ OpenAI 兼容适配器实现（V1 唯一实现）
- ✅ 模型列表获取与缓存（永久缓存，手动刷新）
- ✅ 模型分类标签（内置维护：text-generation、image-generation、video-generation、audio-generation）
- ✅ 同步调用接口
- ✅ 流式调用接口（SSE）
- ✅ 使用记录保存
- ✅ 多 Provider 排序机制（用户拖拽）
- ✅ Provider 启用/禁用开关
- ✅ 并发调用支持（不排队）
- ✅ 可配置重试策略（应用层配置）

**不包含**（明确说明原因）：
- ❌ 其他厂商适配器（Claude、通义千问等）→ V4 云端增强时实现
- ❌ 自动切换到备用 Provider → 应用层自己决定策略
- ❌ 连接池管理 → V1 简化实现，直接调用
- ❌ 流式输出断点续传 → 复杂度高，V2 考虑
- ❌ Provider 健康检查 → V2 监控功能
- ❌ 请求排队机制 → 不需要，允许并发

## 3. 核心流程

### 3.1 配置 Provider 流程

```
用户打开系统设置
  ↓
点击"Provider 管理"
  ↓
点击"添加 Provider"
  ↓
选择类型：OpenAI 兼容
  ↓
填写配置：
  - 名称：我的 OpenAI
  - Base URL：https://api.openai.com/v1
  - API Key：sk-xxx
  ↓
点击"测试连接"
  ↓
系统调用 Provider API 验证
  ↓
测试成功 → 保存配置
  ↓
自动获取模型列表（首次）
  ↓
模型列表缓存到本地
  ↓
显示在 Provider 列表中
```

### 3.2 调用 Provider 流程

```
应用层（AI 工作台）发起调用
  ↓
选择 Provider（或使用默认排序第一个）
  ↓
选择模型（从缓存的模型列表）
  ↓
构造请求参数（prompt、temperature 等）
  ↓
调用 Provider 适配器
  ↓
适配器转换为 Provider 原生格式
  ↓
发送 HTTP 请求到 Provider API
  ↓
（如果失败）检查重试配置
  ↓
（如果配置了重试）按指数退避重试
  ↓
（如果是流式）逐块返回结果
  ↓
（如果流式中断）保存已接收部分
  ↓
记录使用日志
  ↓
返回结果给应用层
```

### 3.3 刷新模型列表流程

```
用户在 Provider 管理界面
  ↓
选择某个 Provider
  ↓
点击"刷新模型列表"
  ↓
系统调用 Provider API 获取最新模型
  ↓
更新本地缓存
  ↓
重新给模型打分类标签
  ↓
显示"已更新 X 个模型"
```

### 3.4 多 Provider 排序流程

```
用户在 Provider 管理界面
  ↓
看到所有已配置的 Provider 列表
  ↓
拖拽某个 Provider 调整顺序
  ↓
系统保存新的排序
  ↓
应用层使用时，默认选择排序第一个
```

## 4. 数据模型

### 4.1 涉及的数据表

**providers 表**（核心）：
```sql
CREATE TABLE providers (
    id TEXT PRIMARY KEY,              -- UUID
    name TEXT NOT NULL,               -- 用户自定义名称，如"我的 OpenAI"
    provider_type TEXT NOT NULL,      -- 类型：openai_compatible, claude, qwen 等
    base_url TEXT NOT NULL,           -- API Base URL
    api_key TEXT NOT NULL,            -- 加密存储的 API Key
    enabled BOOLEAN DEFAULT TRUE,     -- 是否启用
    sort_order INTEGER DEFAULT 0,     -- 排序权重（越小越靠前）
    config JSON,                      -- 额外配置（重试次数等）
    created_at TIMESTAMP,
    updated_at TIMESTAMP,
    user_id TEXT,                     -- 所属用户
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

**models 表**（缓存）：
```sql
CREATE TABLE models (
    id TEXT PRIMARY KEY,              -- UUID
    provider_id TEXT NOT NULL,        -- 所属 Provider
    model_id TEXT NOT NULL,           -- 模型 ID，如 gpt-4
    model_name TEXT,                  -- 模型显示名称
    capabilities JSON,                -- 能力标签数组：["text-generation"]
    context_length INTEGER,           -- 上下文长度
    is_available BOOLEAN DEFAULT TRUE,-- 是否可用
    synced_at TIMESTAMP,              -- 同步时间
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(provider_id, model_id)
);
```

**provider_usage_logs 表**（使用记录）：
```sql
CREATE TABLE provider_usage_logs (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    request_tokens INTEGER,
    response_tokens INTEGER,
    total_tokens INTEGER,
    latency_ms INTEGER,              -- 响应时间（毫秒）
    success BOOLEAN,
    error_code TEXT,                 -- 如果失败，错误码
    created_at TIMESTAMP,
    FOREIGN KEY (provider_id) REFERENCES providers(id),
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

### 4.2 读写关系

**读操作**：
- 获取 Provider 列表：`SELECT * FROM providers WHERE user_id = ? ORDER BY sort_order`
- 获取模型列表：`SELECT * FROM models WHERE provider_id = ? AND is_available = TRUE`
- 查询使用记录：`SELECT * FROM provider_usage_logs WHERE user_id = ? ORDER BY created_at DESC`

**写操作**：
- 创建 Provider：`INSERT INTO providers`
- 更新 Provider：`UPDATE providers SET ... WHERE id = ?`
- 删除 Provider：`DELETE FROM providers WHERE id = ?`（级联删除 models）
- 缓存模型列表：`INSERT OR REPLACE INTO models`
- 记录使用日志：`INSERT INTO provider_usage_logs`

## 5. 验收标准

### 5.1 AC 逐项测试设计表

#### AC01 Provider 正常创建

Given 用户已登录

When 创建 OpenAI Provider 配置

Then Provider 保存成功，模型列表自动获取

#### AC02 API Key 无效检测

Given 用户输入错误的 API Key

When 点击"测试连接"

Then 返回 401 错误，提示"API Key 无效"

#### AC03 模型列表获取与缓存

Given Provider 已配置

When 首次获取模型列表

Then 调用 Provider API，缓存到 models 表

#### AC04 模型分类标签正确

Given 模型列表已获取

When 查询模型

Then gpt-4 标签为 text-generation，dall-e-3 为 image-generation

#### AC05 同步调用成功

Given Provider 和模型已配置

When 调用 chat.completions

Then 返回完整响应，记录使用日志

#### AC06 流式调用成功

Given Provider 和模型已配置

When 调用 chat.completions（stream=true）

Then 返回 SSE 流，逐块接收

#### AC07 流式输出中断处理

Given 流式调用进行中

When 网络中断或 Provider 断开

Then 保存已接收部分，返回错误码

#### AC08 并发调用支持

Given 多个应用同时调用

When 5 个并发请求同时发起

Then 所有请求都能成功处理，无排队

#### AC09 可配置重试策略

Given Provider 配置了 retry_count=3

When API 调用返回 500

Then 自动重试 3 次，间隔 1s/2s/4s

#### AC10 重试失败后返回错误

Given Provider 配置了 retry_count=3

When API 调用 3 次都失败

Then 返回错误，不再重试

#### AC11 多 Provider 排序

Given 用户配置了 3 个 Provider

When 拖拽调整排序

Then 排序保存，默认使用排序第一个

#### AC12 Provider 启用/禁用

Given Provider 已配置

When 禁用某个 Provider

Then 该 Provider 不出现在可选列表

#### AC13 手动刷新模型列表

Given 模型列表已缓存

When 点击"刷新模型列表"

Then 重新获取，更新缓存

#### AC14 使用记录保存

Given 调用成功

When 查询使用记录

Then 包含 tokens、延迟、时间

#### AC 逐项测试设计

证据状态按 spec-docs-pro 词表记录：未创建 / 已创建 / 静态核验 / 本地通过 / 预发通过 / 生产通过 / 失败 / 不适用。原“初始资产状态”列的内容移入“备注”，不代表升格。

| AC | 验收重点 | 测试层级 | 目标资产 | 目标命令 | 证据状态 | 备注 |
| --- | --- | --- | --- | --- | --- | --- |
| AC01 | Provider 正常创建 | 后端 API + 数据库 | `crates/dgos-db/src/provider_scope_tests.rs::provider_and_initial_models_commit_together` | `cargo test --offline -p dgos-db provider_and_initial_models_commit_together` | 已创建 | 已有自动化资产（覆盖原子保存，不覆盖真实 Provider 自动拉取） |
| AC02 | API Key 无效检测 | 后端 API | `crates/dgos-providers/tests/provider_connection_tests.rs` | `cargo test --offline -p dgos-providers invalid_api_key` | 未创建 | 无对应自动化测试 |
| AC03 | 模型列表获取与缓存 | 后端 API + 数据库 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP） |
| AC04 | 模型分类标签正确 | 后端逻辑 | `crates/dgos-providers/src/adapters/openai.rs::test_classify_model` | `cargo test --offline -p dgos-providers test_classify_model` | 已创建 | 已有自动化资产 |
| AC05 | 同步调用成功 | 后端 API + 外部集成 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP，非真实 Provider） |
| AC06 | 流式调用成功 | 后端 API + 外部集成 | `crates/dgos-providers/tests/stream_call_tests.rs` | `cargo test --offline -p dgos-providers stream_call` | 未创建 | 无对应自动化测试 |
| AC07 | 流式输出中断处理 | 后端 API | `crates/dgos-providers/src/manager.rs::cancellable_stream_emits_one_interruption_without_replay` | `cargo test --offline -p dgos-providers cancellable_stream_emits_one_interruption_without_replay` | 已创建 | 已有自动化资产（仅覆盖取消产生一次中断，不覆盖已接收内容的保存） |
| AC08 | 并发调用支持 | 后端 API + 并发测试 | `crates/dgos-providers/tests/concurrent_call_tests.rs` | `cargo test --offline -p dgos-providers concurrent_calls` | 未创建 | 无对应自动化测试 |
| AC09 | 可配置重试策略 | 后端逻辑 | `crates/dgos-providers/src/retry.rs::test_retry_success_after_failures` | `cargo test --offline -p dgos-providers test_retry_success_after_failures` | 已创建 | 已有自动化资产（覆盖重试，不覆盖指定 1s/2s/4s 间隔） |
| AC10 | 重试失败后返回错误 | 后端逻辑 | `crates/dgos-providers/src/retry.rs::test_retry_exhausted` | `cargo test --offline -p dgos-providers test_retry_exhausted` | 已创建 | 已有自动化资产 |
| AC11 | 多 Provider 排序 | 后端 API + 前端集成 | `crates/dgos-db/src/provider_scope_tests.rs::scoped_provider_crud_and_transactional_order_preserve_normal_lifecycle` | `cargo test --offline -p dgos-db scoped_provider_crud_and_transactional_order_preserve_normal_lifecycle` | 已创建 | 已有自动化资产（覆盖事务排序，不覆盖拖拽 UI） |
| AC12 | Provider 启用/禁用 | 后端 API | `tests/providers.test.tsx::uses the dedicated toggle command and updates both provider collections` | `npm test -- --run tests/providers.test.tsx` | 已创建 | 已有自动化资产（Vitest IPC mock） |
| AC13 | 手动刷新模型列表 | 后端 API + 数据库 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP） |
| AC14 | 使用记录保存 | 后端 API + 数据库 | `crates/dgos-providers/src/credential_tests.rs::normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | `cargo test --offline -p dgos-providers normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision` | 已创建 | 已有自动化资产（mock HTTP，检查记录查询及调用 provenance） |


### 5.2 边界条件测试

| 边界条件 | 测试场景 | 预期结果 |
| --- | --- | --- |
| 空 Provider 列表 | 用户未配置任何 Provider | 提示"请先配置 Provider" |
| 重复 Provider 名称 | 创建同名 Provider | 允许（名称可重复，ID 不重复） |
| API Key 包含空白 | 粘贴 key 时前后有空格、换行 | 自动 trim 前后空白；内部空白或非 ASCII 不作为 Bearer key 保存 |
| 超长 Prompt | Prompt 长度超过模型限制 | 返回错误"超过最大长度" |
| 并发超过 100 | 100+ 并发请求 | 正常处理，不崩溃 |
| 网络完全断开 | 调用时无网络 | 返回"网络错误"，不无限等待 |

## 6. 接口设计

本节命令签名为早期草案；当前已注册命令见 `src-tauri/src/lib.rs`、`src-tauri/src/commands/provider_commands.rs` 和 [接口契约 §4.0](../../../04-技术架构/当前版本/V1-接口契约.md#40-当前桌面实施契约d023)。

### 6.1 Tauri Commands（桌面端）

```rust
// Provider CRUD
#[tauri::command]
async fn list_providers(user_id: String) -> Result<Vec<Provider>>;

#[tauri::command]
async fn create_provider(config: ProviderConfig) -> Result<Provider>;

#[tauri::command]
async fn update_provider(id: String, config: ProviderConfig) -> Result<Provider>;

#[tauri::command]
async fn delete_provider(id: String) -> Result<()>;

#[tauri::command]
async fn test_provider_connection(config: ProviderConfig) -> Result<bool>;

// 模型管理
#[tauri::command]
async fn list_models(provider_id: String) -> Result<Vec<Model>>;

#[tauri::command]
async fn refresh_models(provider_id: String) -> Result<Vec<Model>>;

// Provider 调用
#[tauri::command]
async fn call_provider(
    provider_id: String,
    model_id: String,
    request: ChatRequest
) -> Result<ChatResponse>;

#[tauri::command]
async fn stream_provider(
    provider_id: String,
    model_id: String,
    request: ChatRequest
) -> Result<Stream<ChatChunk>>;

// 排序管理
#[tauri::command]
async fn update_provider_order(provider_ids: Vec<String>) -> Result<()>;

// 使用记录
#[tauri::command]
async fn list_usage_logs(
    user_id: String,
    limit: Option<usize>
) -> Result<Vec<UsageLog>>;
```

### 6.2 HTTP API（局域网）

```
POST /api/v1/providers                    # 创建 Provider
GET  /api/v1/providers                    # 获取 Provider 列表
PUT  /api/v1/providers/:id                # 更新 Provider
DELETE /api/v1/providers/:id              # 删除 Provider
POST /api/v1/providers/:id/test           # 测试连接

GET  /api/v1/providers/:id/models         # 获取模型列表
POST /api/v1/providers/:id/models/refresh # 刷新模型列表

POST /api/v1/providers/:id/call           # 调用 Provider（同步）
POST /api/v1/providers/:id/stream         # 调用 Provider（流式）

PUT  /api/v1/providers/order              # 更新排序

GET  /api/v1/usage-logs                   # 使用记录
```

详见 [V1 接口契约](../../../04-技术架构/当前版本/V1-接口契约.md)。

## 7. 技术设计

### 7.1 核心 Trait 定义

```rust
// crates/dgos-providers/src/traits.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// 获取 Provider 类型标识
    fn provider_type(&self) -> &str;
    
    /// 获取模型列表
    async fn list_models(&self) -> Result<Vec<Model>>;
    
    /// 同步调用
    async fn call(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<ChatResponse>;
    
    /// 流式调用
    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<Stream<ChatChunk>>;
    
    /// 测试连接
    async fn test_connection(&self) -> Result<bool>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<Capability>,
    pub context_length: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Capability {
    TextGeneration,
    ImageGeneration,
    VideoGeneration,
    AudioGeneration,
}
```

### 7.2 OpenAI 适配器实现

```rust
// crates/dgos-providers/src/adapters/openai.rs

pub struct OpenAIAdapter {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    retry_config: Option<RetryConfig>,
}

impl OpenAIAdapter {
    pub fn new(base_url: String, api_key: String, retry_config: Option<RetryConfig>) -> Self {
        Self {
            base_url,
            api_key,
            client: reqwest::Client::new(),
            retry_config,
        }
    }
    
    // B01-26: 给模型打分类标签（内置维护）
    fn classify_model(&self, model_id: &str) -> Vec<Capability> {
        match model_id {
            "gpt-4" | "gpt-3.5-turbo" | "gpt-4-turbo" => {
                vec![Capability::TextGeneration]
            }
            "dall-e-3" | "dall-e-2" => {
                vec![Capability::ImageGeneration]
            }
            _ => vec![],
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAIAdapter {
    fn provider_type(&self) -> &str {
        "openai_compatible"
    }
    
    // B01-27: 获取模型列表
    async fn list_models(&self) -> Result<Vec<Model>> {
        let url = format!("{}/models", self.base_url);
        let response = self.client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        
        let models_data: OpenAIModelsResponse = response.json().await?;
        
        let models = models_data.data.into_iter()
            .map(|m| Model {
                id: m.id.clone(),
                name: m.id,
                capabilities: self.classify_model(&m.id), // B01-26
                context_length: None,
            })
            .collect();
        
        Ok(models)
    }
    
    // B01-28: 同步调用（支持重试）
    async fn call(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = OpenAIChatRequest {
            model: model_id.to_string(),
            messages: request.messages,
            temperature: request.temperature,
            stream: false,
        };
        
        // B01-29: 重试逻辑
        let mut attempts = 0;
        let max_attempts = self.retry_config
            .as_ref()
            .map(|c| c.max_retries + 1)
            .unwrap_or(1);
        
        loop {
            attempts += 1;
            
            let response = self.client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;
            
            match response {
                Ok(resp) if resp.status().is_success() => {
                    let data: OpenAIChatResponse = resp.json().await?;
                    return Ok(ChatResponse {
                        content: data.choices[0].message.content.clone(),
                        tokens: data.usage.total_tokens,
                    });
                }
                Ok(resp) if resp.status() == 500 && attempts < max_attempts => {
                    // B01-30: 500 错误重试，指数退避
                    let delay = 2u64.pow(attempts - 1);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                Ok(resp) => {
                    // B01-31: 其他错误不重试
                    return Err(Error::ProviderError(resp.status()));
                }
                Err(e) if attempts < max_attempts => {
                    // B01-32: 网络错误重试
                    let delay = 2u64.pow(attempts - 1);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                Err(e) => {
                    return Err(Error::NetworkError(e));
                }
            }
        }
    }
    
    // B01-33: 流式调用
    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest
    ) -> Result<Stream<ChatChunk>> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = OpenAIChatRequest {
            model: model_id.to_string(),
            messages: request.messages,
            temperature: request.temperature,
            stream: true,
        };
        
        let response = self.client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        
        // B01-34: 转换为 SSE Stream
        let stream = response
            .bytes_stream()
            .map(|chunk| {
                // 解析 SSE 格式
                parse_sse_chunk(chunk)
            });
        
        Ok(stream)
    }
    
    // B01-35: 测试连接
    async fn test_connection(&self) -> Result<bool> {
        let url = format!("{}/models", self.base_url);
        let response = self.client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        
        Ok(response.status().is_success())
    }
}
```

### 7.3 分支到测试追踪表

本节是适配器层分支，编号 B01-26～B01-35；管理器层的同步与流式分支见 §10.2。编号规则与改号对照见 [§10.2 分支编号说明](#分支编号说明2026-10-29)。

| 分支 ID | 分支描述 | AC 映射 | E2E 映射 | NFR 映射 |
| --- | --- | --- | --- | --- |
| B01-26 | 模型分类标签 | AC04 | - | - |
| B01-27 | 获取模型列表 | AC03 | - | - |
| B01-28 | 同步调用 | AC05 | V1-E2E-01 | - |
| B01-29～32 | 重试逻辑 | AC09, AC10 | - | V1-NFR-001（可用性） |
| B01-30 | 500 错误重试 | AC09 | - | - |
| B01-31 | 其他错误不重试 | AC10 | - | - |
| B01-32 | 网络错误重试 | AC09 | - | - |
| B01-33 | 流式调用 | AC06 | V1-E2E-01 | - |
| B01-34 | SSE Stream 解析 | AC06, AC07 | - | - |
| B01-35 | 测试连接 | AC02 | - | - |

## 8. 错误码设计

| 错误码 | 错误信息 | 场景 | 用户操作建议 |
| --- | --- | --- | --- |
| 1001001 | provider_not_found | Provider 不存在 | 检查 Provider ID |
| 1001002 | invalid_api_key | API Key 无效 | 检查 API Key 是否正确 |
| 1001003 | provider_unavailable | Provider 不可用 | 检查网络或 Provider 状态 |
| 1001004 | model_not_found | 模型不存在 | 刷新模型列表或检查模型 ID |
| 1001005 | rate_limit_exceeded | 超过速率限制 | 稍后重试或切换 Provider |
| 1001006 | timeout | 请求超时 | 检查网络或重试 |
| 1001007 | stream_interrupted | 流式输出中断 | 重新生成 |
| 1001008 | max_retries_exceeded | 重试次数耗尽 | 检查 Provider 状态或配置 |

本表错误码编号与 `crates/dgos-types/src/error.rs::error_code()` 一致。错误码文档中曾登记的另一套含义已按 [D038](../../../06-决策记录/V1-冻结决策.md#d038-错误码以代码已发出的映射为准) 作废。1001003 也用于 Provider 已禁用和连接测试失败；另有 1001012（配置或模型刷新已变化）、1001015/1001016（凭据，D023）和通用码 1000000，完整列表见 [V1 错误码 §3.1](../../../04-技术架构/当前版本/V1-错误码.md#31-provider-适配框架01)。

## 9. 非功能需求

### 9.1 性能需求

| 指标 | 目标 | 测量方式 |
| --- | --- | --- |
| 模型列表获取 | < 3 秒 | 集成测试 |
| 同步调用响应 | < 30 秒（取决于 Provider） | 集成测试 |
| 流式首字节延迟 | < 2 秒 | 集成测试 |
| 并发调用支持 | > 100 QPS | 压力测试 |
| 数据库查询 | < 50ms（Provider 列表） | 单元测试 |

### 9.2 安全需求

| 需求 | 实现方式 |
| --- | --- |
| API Key 安全存储 | D023：OS vault 保存 secret，SQLite 仅引用；旧活动明文字段显式迁移，失败保留供重试 |
| HTTPS 强制 | 调用 Provider 时强制 HTTPS |
| 敏感信息脱敏 | D023：不记录 API Key，包括前缀；日志只记录类别与资源引用 |

### 9.3 可用性需求

| 需求 | 实现方式 |
| --- | --- |
| 失败重试 | 可配置重试次数和间隔 |
| 错误提示 | 用户友好的错误信息 |
| 流式中断恢复 | 保存已接收部分，提示重新生成 |

## 10. 实现与验证

### 10.1 验证记录

**说明**：每次验证后更新本节，记录日期、命令、通过率和覆盖说明。格式遵循 spec-docs-pro 标准。
**状态与证据**：实现状态及测试通过证据仅维护在 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)，本节命令仅列验证目标，不代表已执行或通过。

#### 2026-10-26 后端单元测试

```bash
cargo test --offline -p dgos-providers --lib
```

**状态**：🔴 未开始

**目标覆盖**：Provider trait、模型分类、凭据边界及重试策略；具体自动化资产见 §5.1。

---

#### 2026-10-26 后端集成测试

```bash
cargo test --offline -p dgos-db provider_scope_tests
```

**状态**：🔴 未开始

**目标覆盖**：Provider/模型持久化与刷新；其余 AC 的现有资产及缺口见 §5.1。

---

#### 2026-10-26 前端集成测试

```bash
npm test -- --run tests/providers.test.tsx tests/provider-management.test.tsx
```

**状态**：🔴 未开始

**目标覆盖**：Provider 管理页面和 store 的现有 Vitest 覆盖；具体资产见 §5.1。

---

#### 2026-10-26 E2E 测试

**资产状态**：V1-E2E-01 为规划资产（未建立）；当前项目没有 E2E 测试框架，暂无可执行命令。

**状态**：🔴 未开始

**计划覆盖**：
- **V1-E2E-01: 完整配置 Provider 流程**
  1. 启动应用，登录
  2. 打开系统设置 → Provider 管理
  3. 点击"添加 Provider"
  4. 选择"OpenAI 兼容"
  5. 填写：名称="测试 OpenAI"、Base URL、API Key
  6. 点击"测试连接"→ 验证成功提示
  7. 保存配置
  8. 验证 Provider 出现在列表中
  9. 验证模型列表已自动获取
  10. 验证数据库记录正确保存

---

### 10.2 技术设计与分支追踪

#### 技术设计伪代码

**同步调用流程**：

```text
FUNCTION call_provider(provider_id, model_id, request) -> Result<Response>:
  // B01-01: 查询 Provider 配置
  provider = DB.query("SELECT * FROM providers WHERE id = ? AND enabled = TRUE", provider_id)
  IF NOT provider:
    RETURN Err(E_PROVIDER_NOT_FOUND)
  
  // B01-02: 构造适配器
  adapter = create_adapter(provider.provider_type, provider.base_url, provider.api_key)
  native_request = adapter.convert_request(model_id, request)
  
  // B01-03: 读取重试配置
  retry_config = parse_json(provider.config)
  max_retries = retry_config.retry_count OR 0
  
  // B01-04: 发起调用（带重试）
  attempt = 0
  WHILE attempt <= max_retries:
    TRY:
      start_time = now()
      response = HTTP.post(provider.base_url + "/chat/completions", native_request)
      latency_ms = now() - start_time
      
      // B01-05: 检查成功响应
      IF response.status == 200:
        result = adapter.convert_response(response.body)
        
        // B01-06: 记录使用日志
        INSERT INTO provider_usage_logs(
          provider_id, model_id, user_id,
          request_tokens, response_tokens, total_tokens,
          latency_ms, success = TRUE
        )
        
        RETURN Ok(result)
      
      // B01-07: 401 错误（API Key 无效）
      IF response.status == 401:
        RETURN Err(E_INVALID_API_KEY)  // 不重试
      
      // B01-08: 429 错误（速率限制）
      IF response.status == 429:
        IF attempt < max_retries:
          backoff_delay = calculate_backoff(attempt)  // 1s, 2s, 4s, 8s
          SLEEP(backoff_delay)
          attempt += 1
          CONTINUE
        ELSE:
          RETURN Err(E_RATE_LIMIT_EXCEEDED)
      
      // B01-09: 5xx 错误（服务器错误）
      IF response.status >= 500:
        IF attempt < max_retries:
          backoff_delay = calculate_backoff(attempt)
          SLEEP(backoff_delay)
          attempt += 1
          CONTINUE
        ELSE:
          RETURN Err(E_SERVER_ERROR)
      
      // B01-10: 其他错误
      RETURN Err(E_UNKNOWN_ERROR)
    
    CATCH (NetworkError as e):
      // B01-11: 网络错误
      IF attempt < max_retries:
        backoff_delay = calculate_backoff(attempt)
        SLEEP(backoff_delay)
        attempt += 1
        CONTINUE
      ELSE:
        RETURN Err(E_NETWORK_ERROR)
  
  RETURN Err(E_RETRY_EXHAUSTED)
```

**流式调用流程**：

```text
FUNCTION stream_provider(provider_id, model_id, request) -> Result<Stream>:
  // B01-18: 查询 Provider（复用 B01-01 逻辑）
  provider = DB.query("SELECT * FROM providers WHERE id = ? AND enabled = TRUE", provider_id)
  IF NOT provider:
    RETURN Err(E_PROVIDER_NOT_FOUND)
  
  // B01-19: 构造流式请求
  adapter = create_adapter(provider.provider_type, provider.base_url, provider.api_key)
  native_request = adapter.convert_request(model_id, request)
  native_request.stream = TRUE
  
// B01-20: 发起 SSE 流式调用
  TRY:
    sse_stream = HTTP.post_stream(provider.base_url + "/chat/completions", native_request)
    accumulated_text = ""
    
    // B01-21: 返回流处理器
    RETURN StreamHandler {
      on_chunk: FUNCTION(chunk):
        // B01-22: 解析 SSE 数据
        IF chunk.starts_with("data: "):
          data_str = chunk.trim_prefix("data: ")
          IF data_str == "[DONE]":
            // B01-23: 流正常结束
            INSERT INTO provider_usage_logs(success = TRUE, response_tokens = count_tokens(accumulated_text))
            RETURN StreamEnd
          
          data = parse_json(data_str)
          delta = adapter.convert_stream_chunk(data)
          accumulated_text += delta.content
          YIELD delta
      
      on_error: FUNCTION(error):
        // B01-24: 流中断处理
        LOG.warn("Stream interrupted", error)
        
        // B01-25: 保存已接收部分
        INSERT INTO provider_usage_logs(
          provider_id, model_id, user_id,
          response_tokens = count_tokens(accumulated_text),
          success = FALSE,
          error_code = "stream_interrupted"
        )
        
        RETURN Err(E_STREAM_INTERRUPTED, accumulated_text)
    }
  
  CATCH (error):
    RETURN Err(E_NETWORK_ERROR)
```

#### 分支到测试追踪表

##### 分支编号说明（2026-10-29）

分支 ID 前缀是功能编号（FR-001 → `B01`），见[功能文档规范 §8.3](../../功能文档规范.md#83-分支追踪规则)。本文原有两处编号冲突：§10.2 的流式流程使用了 FR-002 的 `B02-01～B02-08`（FR-002 已将其保留给旧规格），§7.2 适配器又把 `B01-01～B01-10` 用作另一组含义。处理方式如下，旧 ID 不再在本功能中使用：

- 本节同步流程的 `B01-01～B01-11` 保持不变（风险说明与检查清单引用的是这一组）。
- `B01-12～B01-17` 已被[归档的旧 AC 追踪表](../../../99-历史归档/V1-追踪矩阵/2026-10-28-旧AC追踪表.md)占用，跳过不用。
- 本节流式流程：原 `B02-01～B02-08` → `B01-18～B01-25`（按原顺序一一对应）。
- §7.2/§7.3 适配器：原 `B01-01～B01-10` → `B01-26～B01-35`（按原顺序一一对应）。
- 下一个可用编号为 `B01-36`。代码、测试中没有引用这些分支 ID。

| 分支编号 | 分支描述 | AC 映射 | E2E 映射 | NFR 映射 | 风险级别 |
|---------|---------|---------|---------|---------|---------|
| B01-01 | 查询 Provider 配置 | AC01, AC12 | V1-E2E-01 | - | 低 |
| B01-02 | 构造适配器与请求 | AC05 | V1-E2E-01 | - | 低 |
| B01-03 | 读取重试配置 | AC09 | - | - | 低 |
| B01-04 | 重试循环主逻辑 | AC09, AC10 | - | V1-NFR-001 | 🔴 高 |
| B01-05 | 检查成功响应 | AC05 | V1-E2E-01 | - | 低 |
| B01-06 | 记录使用日志 | AC14 | - | V1-NFR-002 | 低 |
| B01-07 | 401 错误处理 | AC02 | - | - | 中 |
| B01-08 | 429 速率限制重试 | AC09 | - | V1-NFR-001 | 🔴 高 |
| B01-09 | 5xx 服务器错误重试 | AC09, AC10 | - | V1-NFR-001 | 🔴 高 |
| B01-10 | 其他错误处理 | AC05 | - | - | 中 |
| B01-11 | 网络错误重试 | AC09 | - | V1-NFR-001 | 🔴 高 |
| B01-18 | 查询 Provider（流式） | AC12 | - | - | 低 |
| B01-19 | 构造流式请求 | AC06 | - | - | 低 |
| B01-20 | 发起 SSE 调用 | AC06 | V1-E2E-01 | - | 中 |
| B01-21～B01-22 | 流处理与 SSE 解析 | AC06 | V1-E2E-01 | V1-NFR-003 | 中 |
| B01-23 | 流正常结束 | AC06 | - | - | 低 |
| B01-24～B01-25 | 流中断保存已接收 | AC07 | - | V1-NFR-001 | 🔴 高 |

**高风险分支说明**：
- 🔴 **B01-04**: 重试循环是可靠性核心，必须充分测试边界（0次、1次、3次重试）
- 🔴 **B01-08, B01-09, B01-11**: 各种错误的重试逻辑，容易出现死循环或过早放弃
- 🔴 **B01-24～B01-25**: 流中断时必须保存已接收部分，否则用户看到的内容会丢失

**NFR 映射说明**：
- **V1-NFR-001 (可靠性)**: 重试机制、错误恢复
- **V1-NFR-002 (可观测性)**: 使用日志记录
- **V1-NFR-003 (性能)**: 流式响应性能

---

### 10.3 验证检查清单

开发完成后，必须通过以下检查才能合并：

**代码检查**：
- [ ] 所有 AC 的目标测试文件已创建
- [ ] 高风险分支（B01-04, B01-08, B01-09, B01-11, B01-24～B01-25）有独立测试用例
- [ ] 重试逻辑有完整的单元测试（0次、1次、3次、边界条件）
- [ ] 流中断场景有明确的回归测试

**测试检查**：
- [ ] 后端集成测试通过率 ≥ 80%（mock 外部 API 的场景可延后）
- [ ] 并发测试验证无竞态条件（5个并发、100个并发）
- [ ] E2E 测试覆盖完整用户流程（V1-E2E-01）

**数据检查**：
- [ ] 使用记录正确保存到数据库（包含 tokens、latency）
- [ ] API Key 加密存储验证通过（bcrypt 或 AES）
- [ ] 模型列表缓存正确（首次同步、手动刷新）

**文档检查**：
- [ ] 验证记录章节已更新（记录日期、通过率、覆盖说明）
- [ ] 所有高风险分支的测试结果已记录

## 11. 风险与依赖

### 11.1 技术风险

| 风险 | 概率 | 影响 | 缓解措施 |
| --- | --- | --- | --- |
| OpenAI API 变更 | 中 | 高 | 版本锁定，监控 API 变更 |
| 流式输出不稳定 | 中 | 中 | 完善错误处理，保存已接收部分 |
| 并发性能瓶颈 | 低 | 中 | 压力测试，必要时引入连接池 |
| API Key 泄漏 | 低 | 高 | 加密存储，日志脱敏 |

### 11.2 外部依赖

| 依赖 | 类型 | 风险 | 应对 |
| --- | --- | --- | --- |
| OpenAI API | 外部服务 | 可用性依赖 | 用户自己配置，支持多 Provider |
| reqwest | Rust Crate | 版本兼容 | 锁定版本 |
| tokio | Rust Crate | 版本兼容 | 锁定版本 |

### 11.3 功能依赖

| 依赖功能 | 状态 | 影响 |
| --- | --- | --- |
| 本地用户管理（V1-FR-008） | 非前置（D023） | 当前使用宿主派生的 `default-user`；完整多用户隔离随 FR-008 验收 |
| SQLite 数据库 | 迁移依赖 | Provider 相关表由 migration 建立，应用启动时自动迁移 |

## 12. 实现状态

只在 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md) 维护功能状态；本节不复制进度或百分比。

## 13. 变更记录

| 日期 | 变更内容 | 变更人 |
| --- | --- | --- |
| 2024-01-XX | 初始版本，基于 spec-docs-pro 创建 | Kiro |
| 2026-10-29 | §8 按 D038 说明与错误码文档的关系；解决分支 ID 冲突（流式 B02-01～08 → B01-18～25，适配器 B01-01～10 → B01-26～35）；§11.3/§12 删除与 D023 及实现状态矛盾的“规划中/未开始”表述 | Kiro |

---

**文档状态**：持续维护；实现状态以 V1 实现状态为准
