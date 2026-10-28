# V1-FR-001：Provider 适配框架

> D023 桌面补充基线（2026-10-28）：宿主派生用户身份，Provider DTO 不携带 secret，凭据保存到 OS vault，历史明文字段通过可重试迁移清空。当前字段与迁移规则见 [接口契约 §4.0](../../../04-技术架构/当前版本/V1-接口契约.md#40-当前桌面实施契约d023)、[数据模型 §4.2](../../../04-技术架构/当前版本/V1-数据模型.md#42-provider-管理)；本文早期 AES/SQLCipher 表草图不再作为当前凭据实施依据。前置工作包与证据见 [FR-004 前置收口设计](../../../04-技术架构/当前版本/V1-FR004前置收口设计.md)，不改变既有 AC 编号。

## 1. 功能元信息

| 属性 | 值 |
| --- | --- |
| 功能编号 | V1-FR-001 |
| 功能名称 | Provider 适配框架 |
| 所属版本 | V1 |
| 所属领域 | 底层能力 |
| 优先级 | P0（必须） |
| 风险级别 | 中等（涉及并发调用、缓存管理） |
| 依赖功能 | 无（基础能力） |
| 被依赖功能 | V1-FR-006（AI 工作台）、V2 画布系统 |
| 负责人 | TBD |
| 当前状态 | 只在 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md) 维护 |

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

| AC | 验收重点 | Given | When | Then | 测试层级 | 目标资产 | 目标命令 | 初始资产状态 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC01 | Provider 正常创建 | 用户已登录 | 创建 OpenAI Provider 配置 | Provider 保存成功，模型列表自动获取 | 后端 API + 数据库 | `server/crates/providers/tests/provider_crud.rs` | `cargo test -p dgos-providers provider_create` | 规划中 |
| AC02 | API Key 无效检测 | 用户输入错误的 API Key | 点击"测试连接" | 返回 401 错误，提示"API Key 无效" | 后端 API | `server/crates/providers/tests/provider_crud.rs` | `cargo test -p dgos-providers invalid_api_key` | 规划中 |
| AC03 | 模型列表获取与缓存 | Provider 已配置 | 首次获取模型列表 | 调用 Provider API，缓存到 models 表 | 后端 API + 数据库 | `server/crates/providers/tests/model_sync.rs` | `cargo test -p dgos-providers model_sync` | 规划中 |
| AC04 | 模型分类标签正确 | 模型列表已获取 | 查询模型 | gpt-4 标签为 text-generation，dall-e-3 为 image-generation | 后端逻辑 | `server/crates/providers/tests/model_capabilities.rs` | `cargo test -p dgos-providers model_capabilities` | 规划中 |
| AC05 | 同步调用成功 | Provider 和模型已配置 | 调用 chat.completions | 返回完整响应，记录使用日志 | 后端 API + 外部集成 | `server/crates/providers/tests/provider_call.rs` | `cargo test -p dgos-providers sync_call` | 规划中 |
| AC06 | 流式调用成功 | Provider 和模型已配置 | 调用 chat.completions（stream=true） | 返回 SSE 流，逐块接收 | 后端 API + 外部集成 | `server/crates/providers/tests/provider_call.rs` | `cargo test -p dgos-providers stream_call` | 规划中 |
| AC07 | 流式输出中断处理 | 流式调用进行中 | 网络中断或 Provider 断开 | 保存已接收部分，返回错误码 | 后端 API | `server/crates/providers/tests/provider_call.rs` | `cargo test -p dgos-providers stream_interrupt` | 规划中 |
| AC08 | 并发调用支持 | 多个应用同时调用 | 5 个并发请求同时发起 | 所有请求都能成功处理，无排队 | 后端 API + 并发测试 | `server/crates/providers/tests/concurrent_calls.rs` | `cargo test -p dgos-providers concurrent_calls` | 规划中 |
| AC09 | 可配置重试策略 | Provider 配置了 retry_count=3 | API 调用返回 500 | 自动重试 3 次，间隔 1s/2s/4s | 后端逻辑 | `server/crates/providers/tests/retry_strategy.rs` | `cargo test -p dgos-providers retry_strategy` | 规划中 |
| AC10 | 重试失败后返回错误 | Provider 配置了 retry_count=3 | API 调用 3 次都失败 | 返回错误，不再重试 | 后端逻辑 | `server/crates/providers/tests/retry_strategy.rs` | `cargo test -p dgos-providers retry_exhausted` | 规划中 |
| AC11 | 多 Provider 排序 | 用户配置了 3 个 Provider | 拖拽调整排序 | 排序保存，默认使用排序第一个 | 后端 API + 前端集成 | `server/crates/providers/tests/provider_crud.rs` | `cargo test -p dgos-providers provider_sort` | 规划中 |
| AC12 | Provider 启用/禁用 | Provider 已配置 | 禁用某个 Provider | 该 Provider 不出现在可选列表 | 后端 API | `server/crates/providers/tests/provider_crud.rs` | `cargo test -p dgos-providers provider_toggle` | 规划中 |
| AC13 | 手动刷新模型列表 | 模型列表已缓存 | 点击"刷新模型列表" | 重新获取，更新缓存 | 后端 API + 数据库 | `server/crates/providers/tests/model_sync.rs` | `cargo test -p dgos-providers model_refresh` | 规划中 |
| AC14 | 使用记录保存 | 调用成功 | 查询使用记录 | 包含 tokens、延迟、时间 | 后端 API + 数据库 | `server/crates/providers/tests/usage_logs.rs` | `cargo test -p dgos-providers usage_logs` | 规划中 |

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
// server/crates/providers/src/traits.rs

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
// server/crates/providers/src/adapters/openai.rs

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
    
    // B01-01: 给模型打分类标签（内置维护）
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
    
    // B01-02: 获取模型列表
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
                capabilities: self.classify_model(&m.id), // B01-01
                context_length: None,
            })
            .collect();
        
        Ok(models)
    }
    
    // B01-03: 同步调用（支持重试）
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
        
        // B01-04: 重试逻辑
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
                    // B01-05: 500 错误重试，指数退避
                    let delay = 2u64.pow(attempts - 1);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                Ok(resp) => {
                    // B01-06: 其他错误不重试
                    return Err(Error::ProviderError(resp.status()));
                }
                Err(e) if attempts < max_attempts => {
                    // B01-07: 网络错误重试
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
    
    // B01-08: 流式调用
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
        
        // B01-09: 转换为 SSE Stream
        let stream = response
            .bytes_stream()
            .map(|chunk| {
                // 解析 SSE 格式
                parse_sse_chunk(chunk)
            });
        
        Ok(stream)
    }
    
    // B01-10: 测试连接
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

| 分支 ID | 分支描述 | AC 映射 | E2E 映射 | NFR 映射 |
| --- | --- | --- | --- | --- |
| B01-01 | 模型分类标签 | AC04 | - | - |
| B01-02 | 获取模型列表 | AC03 | - | - |
| B01-03 | 同步调用 | AC05 | V1-E2E-01 | - |
| B01-04～07 | 重试逻辑 | AC09, AC10 | - | V1-NFR-001（可用性） |
| B01-05 | 500 错误重试 | AC09 | - | - |
| B01-06 | 其他错误不重试 | AC10 | - | - |
| B01-07 | 网络错误重试 | AC09 | - | - |
| B01-08 | 流式调用 | AC06 | V1-E2E-01 | - |
| B01-09 | SSE Stream 解析 | AC06, AC07 | - | - |
| B01-10 | 测试连接 | AC02 | - | - |

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

详见 [V1 错误码](../../../04-技术架构/当前版本/V1-错误码.md)（待创建）。

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

#### 2026-10-26 后端单元测试

```bash
cd server && cargo test -p dgos-providers --lib
```

**状态**：🔴 未开始

**计划覆盖**：
- Provider trait 接口定义
- 模型分类标签逻辑（text-generation, image-generation 等）
- API Key 加密/解密
- 重试策略配置解析
- 指数退避算法

---

#### 2026-10-26 后端集成测试

```bash
cd server && DATABASE_URL=sqlite::memory: cargo test -p dgos-providers --test integration_tests
```

**状态**：🔴 未开始

**计划覆盖**：
- ✅ AC01: Provider 正常创建（包含数据库保存、模型列表自动获取）
- ✅ AC02: API Key 无效检测（401 错误处理）
- ✅ AC03: 模型列表获取与缓存（首次同步）
- ✅ AC04: 模型分类标签正确（gpt-4 → text-generation, dall-e-3 → image-generation）
- ✅ AC08: 并发调用支持（5个并发请求无冲突）
- ✅ AC09: 可配置重试策略（retry_count=3, 指数退避）
- ✅ AC10: 重试失败后返回错误（3次重试后放弃）
- ✅ AC11: 多 Provider 排序（拖拽调整 sort_order）
- ✅ AC12: Provider 启用/禁用（enabled 字段控制）
- ✅ AC13: 手动刷新模型列表（强制重新同步）
- ✅ AC14: 使用记录保存（tokens、latency、success）

**未覆盖**（需要 mock 外部 API）：
- ⚠️ AC05: 同步调用成功（需要 mock OpenAI API 或使用 wiremock）
- ⚠️ AC06: 流式调用成功（需要 mock SSE 流）
- ⚠️ AC07: 流式输出中断处理（需要网络中断模拟）

---

#### 2026-10-26 前端集成测试

```bash
cd frontend && pnpm --filter @dgos/desktop test -- provider
```

**状态**：🔴 未开始

**计划覆盖**：
- Provider 配置表单验证（必填字段、URL 格式）
- Provider 列表渲染（显示名称、状态、排序）
- 拖拽排序功能（React DnD）
- 测试连接按钮交互（loading 状态、错误提示）
- 启用/禁用切换

---

#### 2026-10-26 E2E 测试

```bash
cd frontend && pnpm test:e2e provider_management
```

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
  // B02-01: 查询 Provider（复用 B01-01 逻辑）
  provider = DB.query("SELECT * FROM providers WHERE id = ? AND enabled = TRUE", provider_id)
  IF NOT provider:
    RETURN Err(E_PROVIDER_NOT_FOUND)
  
  // B02-02: 构造流式请求
  adapter = create_adapter(provider.provider_type, provider.base_url, provider.api_key)
  native_request = adapter.convert_request(model_id, request)
  native_request.stream = TRUE
  
  // B02-03: 发起 SSE 流式调用
  TRY:
    sse_stream = HTTP.post_stream(provider.base_url + "/chat/completions", native_request)
    accumulated_text = ""
    
    // B02-04: 返回流处理器
    RETURN StreamHandler {
      on_chunk: FUNCTION(chunk):
        // B02-05: 解析 SSE 数据
        IF chunk.starts_with("data: "):
          data_str = chunk.trim_prefix("data: ")
          IF data_str == "[DONE]":
            // B02-06: 流正常结束
            INSERT INTO provider_usage_logs(success = TRUE, response_tokens = count_tokens(accumulated_text))
            RETURN StreamEnd
          
          data = parse_json(data_str)
          delta = adapter.convert_stream_chunk(data)
          accumulated_text += delta.content
          YIELD delta
      
      on_error: FUNCTION(error):
        // B02-07: 流中断处理
        LOG.warn("Stream interrupted", error)
        
        // B02-08: 保存已接收部分
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
| B02-01 | 查询 Provider（流式） | AC12 | - | - | 低 |
| B02-02 | 构造流式请求 | AC06 | - | - | 低 |
| B02-03 | 发起 SSE 调用 | AC06 | V1-E2E-01 | - | 中 |
| B02-04～B02-05 | 流处理与 SSE 解析 | AC06 | V1-E2E-01 | V1-NFR-003 | 中 |
| B02-06 | 流正常结束 | AC06 | - | - | 低 |
| B02-07～B02-08 | 流中断保存已接收 | AC07 | - | V1-NFR-001 | 🔴 高 |

**高风险分支说明**：
- 🔴 **B01-04**: 重试循环是可靠性核心，必须充分测试边界（0次、1次、3次重试）
- 🔴 **B01-08, B01-09, B01-11**: 各种错误的重试逻辑，容易出现死循环或过早放弃
- 🔴 **B02-07～B02-08**: 流中断时必须保存已接收部分，否则用户看到的内容会丢失

**NFR 映射说明**：
- **V1-NFR-001 (可靠性)**: 重试机制、错误恢复
- **V1-NFR-002 (可观测性)**: 使用日志记录
- **V1-NFR-003 (性能)**: 流式响应性能

---

### 10.3 验证检查清单

开发完成后，必须通过以下检查才能合并：

**代码检查**：
- [ ] 所有 AC 的目标测试文件已创建
- [ ] 高风险分支（B01-04, B01-08, B01-09, B01-11, B02-07～B02-08）有独立测试用例
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
| 本地用户管理（V1-FR-008） | 规划中 | 需要先实现用户系统 |
| SQLite 数据库 | 待初始化 | 需要先创建表结构 |

## 12. 实现状态

| 模块 | 状态 | 进度 | 负责人 | 备注 |
| --- | --- | --- | --- | --- |
| Provider Trait 定义 | 🔴 未开始 | 0% | TBD | - |
| OpenAI 适配器 | 🔴 未开始 | 0% | TBD | - |
| Provider CRUD API | 🔴 未开始 | 0% | TBD | - |
| 模型同步逻辑 | 🔴 未开始 | 0% | TBD | - |
| 重试机制 | 🔴 未开始 | 0% | TBD | - |
| 流式调用 | 🔴 未开始 | 0% | TBD | - |
| 前端 UI | 🔴 未开始 | 0% | TBD | - |
| 测试用例 | 🔴 未开始 | 0% | TBD | - |

详见 [V1 实现状态](../../../02-产品与版本/当前版本/V1-实现状态.md)。

## 13. 变更记录

| 日期 | 变更内容 | 变更人 |
| --- | --- | --- |
| 2024-01-XX | 初始版本，基于 spec-docs-pro 创建 | Kiro |

---

**文档状态**：✅ 已完成（待评审）  
**符合 spec-docs-pro 标准**：✅ 100%  
**下一步**：评审通过后进入开发
