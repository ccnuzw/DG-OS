# V1-FR-002：Skills 管理系统

## 1. 功能元信息

| 属性 | 值 |
| --- | --- |
| 功能编号 | V1-FR-002 |
| 功能名称 | Skills 管理系统 |
| 所属版本 | V1 |
| 所属领域 | 底层能力 |
| 优先级 | P0（必须） |
| 风险级别 | 标准（使用现有标准，不涉及复杂并发） |
| 依赖功能 | V1-FR-001（Provider 框架，Skills 可能调用 AI） |
| 被依赖功能 | V1-FR-006（AI 工作台）、V2 画布系统 |
| 负责人 | TBD |
| 当前状态 | 规划中 |

## 2. 功能概述

### 2.1 一句话描述

连接官方 Skills 仓库，提供 Skills 浏览、安装、配置、调用和更新能力，支持 HTTP 和 CLI 两种调用方式，扩展 AI 应用的工具能力。

### 2.2 详细说明

Skills 管理系统让 DGOS 能够使用现有的 AI 工具生态（基于 LangChain Tools 标准）。DGOS 不重新实现 Skills 协议，只负责管理和调用。

**核心价值**：
- ✅ 使用现有生态：直接使用 LangChain Tools 生态的 Skills
- ✅ 简化管理：统一的安装、配置、更新界面
- ✅ 扩展能力：让 AI 可以使用工具（搜索、计算、文件操作等）
- ✅ 可选增强：Skills 是可选的，不是必须的

**用户价值**：
- 开发者：可以让 AI 调用各种工具，增强能力
- 最终用户：可以在 AI 工作台或画布中使用 Skills 增强生成效果

**与 Provider 的区别**：
- Provider：提供 AI 模型（GPT-4、Claude 等）
- Skills：提供工具能力（搜索、计算、文件操作等）
- Skills 是可选的增强，Provider 是必须的

### 2.3 功能边界

**包含**：
- ✅ 连接官方 Skills 仓库（硬编码 URL：`https://skills.dgos.ai/registry.json`）
- ✅ 浏览 Skills 列表（分类、搜索、筛选）
- ✅ 查看 Skill 详情（名称、描述、作者、版本、依赖）
- ✅ 安装 Skills（下载到 `~/.dgos/skills/`）
- ✅ 卸载 Skills
- ✅ 启用/禁用 Skills
- ✅ Skills 配置管理（API Key 等敏感配置）
- ✅ 调用 Skills（HTTP 和 CLI 两种方式）
- ✅ Skills 更新检查（提示更新，用户手动确认）
- ✅ 依赖检查（检查 Python/Node.js 版本）
- ✅ 使用记录保存
- ✅ 并发调用支持

**不包含**（明确说明原因）：
- ❌ 自定义 Skills 仓库 → V2 再支持，V1 简化实现
- ❌ Skills 开发 SDK → V8 开发者生态时提供
- ❌ 自动更新 Skills → 需要用户确认，避免意外变更
- ❌ 内置 Python/Node.js 运行时 → 复杂度高，V2 考虑
- ❌ Skills 沙箱隔离 → V2 安全增强时考虑
- ❌ Skills 市场（用户上传） → V3 应用市场时一起做

## 3. 核心流程

### 3.1 浏览和安装 Skills 流程

```
用户打开系统设置
  ↓
点击"Skills 管理"
  ↓
点击"浏览 Skills 仓库"
  ↓
系统从官方仓库获取 Skills 列表
  ↓
显示 Skills 列表（分类：搜索、计算、文件、数据等）
  ↓
用户搜索或浏览
  ↓
点击某个 Skill 查看详情
  ↓
显示：名称、描述、作者、版本、依赖、配置项
  ↓
点击"安装"
  ↓
检查依赖（Python 3.10+、Node.js 18+ 等）
  ↓
（如果依赖不满足）提示用户先安装依赖
  ↓
（如果依赖满足）下载 Skill 到 ~/.dgos/skills/
  ↓
解压并验证完整性
  ↓
（如果需要配置）提示用户配置（API Key 等）
  ↓
安装完成，显示在已安装列表
```

### 3.2 配置 Skills 流程

```
用户在已安装 Skills 列表
  ↓
选择某个 Skill
  ↓
点击"配置"
  ↓
显示配置项（根据 Skill manifest）
  ↓
例如：Google Search Skill 需要 API Key
  ↓
用户输入配置值
  ↓
系统加密保存到数据库
  ↓
配置完成，Skill 可用
```

### 3.3 调用 Skills 流程（HTTP）

```
应用层（AI 工作台）发起调用
  ↓
选择要使用的 Skill（如 web-search）
  ↓
构造调用参数（如 query: "Rust 教程"）
  ↓
Skills 管理器查找 Skill 配置
  ↓
检查 Skill 是否启用
  ↓
检查 Skill 类型：HTTP
  ↓
（如果 Skill 未启动）启动 Skill HTTP 服务
  ↓
发送 HTTP 请求到 Skill
  ↓
Skill 处理并返回结果
  ↓
记录使用日志
  ↓
返回结果给应用层
```

### 3.4 调用 Skills 流程（CLI）

```
应用层发起调用
  ↓
选择要使用的 Skill（如 calculator）
  ↓
构造调用参数（如 expression: "2+2"）
  ↓
Skills 管理器查找 Skill 配置
  ↓
检查 Skill 类型：CLI
  ↓
执行命令：calculator "2+2"
  ↓
从 stdout 读取结果
  ↓
解析 JSON 结果
  ↓
记录使用日志
  ↓
返回结果给应用层
```

### 3.5 更新 Skills 流程

```
系统定期检查 Skills 更新（每天一次）
  ↓
从官方仓库获取最新版本信息
  ↓
对比本地已安装版本
  ↓
（如果有新版本）在 Skills 列表显示"有更新"标记
  ↓
用户点击"查看更新"
  ↓
显示更新日志（changelog）
  ↓
用户点击"更新"
  ↓
下载新版本到临时目录
  ↓
备份旧版本（以便回滚）
  ↓
替换为新版本
  ↓
重启 Skill（如果是 HTTP 服务）
  ↓
更新完成
```

## 4. 数据模型

### 4.1 涉及的数据表

**skills 表**（已安装的 Skills）：
```sql
CREATE TABLE skills (
    id TEXT PRIMARY KEY,              -- UUID
    skill_id TEXT NOT NULL,           -- Skill 标识，如 web-search
    name TEXT NOT NULL,               -- Skill 名称
    description TEXT,                 -- 描述
    version TEXT NOT NULL,            -- 当前安装的版本
    author TEXT,                      -- 作者
    install_path TEXT NOT NULL,       -- 安装路径
    call_type TEXT NOT NULL,          -- 调用方式：http, cli
    enabled BOOLEAN DEFAULT TRUE,     -- 是否启用
    config JSON,                      -- 用户配置（加密存储）
    manifest JSON,                    -- Skill manifest（包含依赖、配置项等）
    installed_at TIMESTAMP,
    updated_at TIMESTAMP,
    user_id TEXT,                     -- 所属用户
    FOREIGN KEY (user_id) REFERENCES users(id),
    UNIQUE(user_id, skill_id)
);
```

**skill_registry_cache 表**（仓库缓存）：
```sql
CREATE TABLE skill_registry_cache (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    latest_version TEXT,
    author TEXT,
    category TEXT,                    -- 分类：search, compute, file, data
    tags JSON,                        -- 标签数组
    download_url TEXT,
    manifest JSON,
    synced_at TIMESTAMP,
    UNIQUE(skill_id)
);
```

**skill_usage_logs 表**（使用记录）：
```sql
CREATE TABLE skill_usage_logs (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    input_params JSON,                -- 输入参数
    output_result JSON,               -- 输出结果（可能很大，考虑截断）
    success BOOLEAN,
    error_message TEXT,
    latency_ms INTEGER,               -- 响应时间
    created_at TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id)
);
```

### 4.2 读写关系

**读操作**：
- 获取已安装 Skills：`SELECT * FROM skills WHERE user_id = ? AND enabled = TRUE`
- 获取仓库 Skills：`SELECT * FROM skill_registry_cache WHERE category = ?`
- 查询使用记录：`SELECT * FROM skill_usage_logs WHERE user_id = ? ORDER BY created_at DESC`

**写操作**：
- 安装 Skill：`INSERT INTO skills`
- 更新 Skill：`UPDATE skills SET version = ?, updated_at = ? WHERE id = ?`
- 卸载 Skill：`DELETE FROM skills WHERE id = ?`
- 缓存仓库：`INSERT OR REPLACE INTO skill_registry_cache`
- 记录使用日志：`INSERT INTO skill_usage_logs`

## 5. 验收标准

### 5.1 AC 逐项测试设计表

| AC | 验收重点 | Given | When | Then | 测试层级 | 目标资产 | 目标命令 | 初始资产状态 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC01 | 连接官方仓库 | 网络正常 | 打开 Skills 浏览 | 成功获取 Skills 列表 | 后端 API + 外部集成 | `server/crates/skills/tests/registry.rs` | `cargo test -p dgos-skills registry_fetch` | 规划中 |
| AC02 | 浏览 Skills 列表 | 仓库已缓存 | 查看 Skills 列表 | 显示所有 Skills，支持分类和搜索 | 后端 API + 前端 | `server/crates/skills/tests/registry.rs` | `cargo test -p dgos-skills skills_list` | 规划中 |
| AC03 | 查看 Skill 详情 | Skill 存在 | 点击某个 Skill | 显示完整信息（名称、描述、版本、依赖） | 后端 API + 前端 | `server/crates/skills/tests/registry.rs` | `cargo test -p dgos-skills skill_detail` | 规划中 |
| AC04 | 依赖检查 | 安装需要 Python 3.10+ | 点击安装 | 检查 Python 版本，不满足时提示 | 后端逻辑 | `server/crates/skills/tests/dependency.rs` | `cargo test -p dgos-skills dependency_check` | 规划中 |
| AC05 | 安装 HTTP Skill | 依赖满足 | 安装 web-search | 下载、解压、保存到数据库 | 后端 API + 文件系统 | `server/crates/skills/tests/installation.rs` | `cargo test -p dgos-skills install_http_skill` | 规划中 |
| AC06 | 安装 CLI Skill | 依赖满足 | 安装 calculator | 下载、解压、保存到数据库 | 后端 API + 文件系统 | `server/crates/skills/tests/installation.rs` | `cargo test -p dgos-skills install_cli_skill` | 规划中 |
| AC07 | Skill 配置 | Skill 需要配置 | 输入 API Key | 加密保存到数据库 | 后端 API + 数据库 | `server/crates/skills/tests/configuration.rs` | `cargo test -p dgos-skills skill_config` | 规划中 |
| AC08 | 调用 HTTP Skill | Skill 已安装 | 调用 web-search("Rust") | 启动服务、发送请求、返回结果 | 后端 API + 外部进程 | `server/crates/skills/tests/skill_call.rs` | `cargo test -p dgos-skills call_http_skill` | 规划中 |
| AC09 | 调用 CLI Skill | Skill 已安装 | 调用 calculator("2+2") | 执行命令、解析输出、返回结果 | 后端 API + 进程管理 | `server/crates/skills/tests/skill_call.rs` | `cargo test -p dgos-skills call_cli_skill` | 规划中 |
| AC10 | 并发调用 | 多个应用同时调用 | 5 个并发调用 web-search | 所有调用都成功，无冲突 | 后端 API + 并发测试 | `server/crates/skills/tests/concurrent.rs` | `cargo test -p dgos-skills concurrent_calls` | 规划中 |
| AC11 | Skill 启用/禁用 | Skill 已安装 | 禁用 Skill | 该 Skill 不出现在可用列表 | 后端 API | `server/crates/skills/tests/installation.rs` | `cargo test -p dgos-skills skill_toggle` | 规划中 |
| AC12 | Skill 卸载 | Skill 已安装 | 卸载 Skill | 删除文件、删除数据库记录 | 后端 API + 文件系统 | `server/crates/skills/tests/installation.rs` | `cargo test -p dgos-skills uninstall_skill` | 规划中 |
| AC13 | 更新检查 | Skill 有新版本 | 检查更新 | 显示"有更新"标记和 changelog | 后端 API | `server/crates/skills/tests/update.rs` | `cargo test -p dgos-skills update_check` | 规划中 |
| AC14 | Skill 更新 | 用户确认更新 | 点击更新 | 下载新版本、备份旧版本、替换 | 后端 API + 文件系统 | `server/crates/skills/tests/update.rs` | `cargo test -p dgos-skills skill_update` | 规划中 |
| AC15 | 使用记录保存 | 调用成功 | 查询使用记录 | 包含输入、输出、延迟 | 后端 API + 数据库 | `server/crates/skills/tests/usage_logs.rs` | `cargo test -p dgos-skills usage_logs` | 规划中 |

### 5.2 边界条件测试

| 边界条件 | 测试场景 | 预期结果 |
| --- | --- | --- |
| 网络断开 | 浏览仓库时无网络 | 使用缓存数据，提示"离线模式" |
| 依赖不满足 | Python 版本低于要求 | 提示"需要 Python 3.10+"，不允许安装 |
| 重复安装 | 安装已安装的 Skill | 提示"已安装"，询问是否重新安装 |
| Skill 调用超时 | HTTP Skill 5 秒未响应 | 返回超时错误 |
| Skill 调用失败 | Skill 返回错误 | 记录错误日志，返回错误信息 |
| 配置缺失 | 调用未配置的 Skill | 提示"请先配置 API Key" |
| 磁盘空间不足 | 安装 Skill 时空间不足 | 返回错误，清理临时文件 |

## 6. 接口设计

### 6.1 Tauri Commands（桌面端）

```rust
// Skills 仓库
#[tauri::command]
async fn fetch_skills_registry() -> Result<Vec<SkillInfo>>;

#[tauri::command]
async fn search_skills(query: String, category: Option<String>) -> Result<Vec<SkillInfo>>;

#[tauri::command]
async fn get_skill_detail(skill_id: String) -> Result<SkillDetail>;

// Skills 安装管理
#[tauri::command]
async fn install_skill(skill_id: String) -> Result<Skill>;

#[tauri::command]
async fn uninstall_skill(skill_id: String) -> Result<()>;

#[tauri::command]
async fn list_installed_skills(user_id: String) -> Result<Vec<Skill>>;

#[tauri::command]
async fn toggle_skill(skill_id: String, enabled: bool) -> Result<()>;

// Skills 配置
#[tauri::command]
async fn get_skill_config(skill_id: String) -> Result<SkillConfig>;

#[tauri::command]
async fn update_skill_config(skill_id: String, config: SkillConfig) -> Result<()>;

// Skills 调用
#[tauri::command]
async fn call_skill(
    skill_id: String,
    params: serde_json::Value
) -> Result<serde_json::Value>;

// Skills 更新
#[tauri::command]
async fn check_skill_updates() -> Result<Vec<SkillUpdate>>;

#[tauri::command]
async fn update_skill(skill_id: String) -> Result<Skill>;

// 依赖检查
#[tauri::command]
async fn check_dependencies(skill_id: String) -> Result<DependencyStatus>;

// 使用记录
#[tauri::command]
async fn list_skill_usage_logs(
    user_id: String,
    limit: Option<usize>
) -> Result<Vec<SkillUsageLog>>;
```

### 6.2 HTTP API（局域网）

```
GET  /api/v1/skills/registry              # 获取仓库 Skills
GET  /api/v1/skills/registry/:id          # 获取 Skill 详情
GET  /api/v1/skills/installed             # 已安装 Skills
POST /api/v1/skills/install               # 安装 Skill
DELETE /api/v1/skills/:id                 # 卸载 Skill
POST /api/v1/skills/:id/call              # 调用 Skill
PUT  /api/v1/skills/:id/config            # 更新配置
POST /api/v1/skills/:id/toggle            # 启用/禁用
GET  /api/v1/skills/updates               # 检查更新
POST /api/v1/skills/:id/update            # 更新 Skill
GET  /api/v1/skills/usage-logs            # 使用记录
```

详见 [V1 接口契约](../../../04-技术架构/当前版本/V1-接口契约.md)。

## 7. 技术设计

### 7.1 Skills Manifest 格式

```json
{
  "id": "web-search",
  "name": "Web Search",
  "version": "1.0.0",
  "description": "Search the web using Google",
  "author": "DGOS Team",
  "category": "search",
  "tags": ["search", "google", "web"],
  "call_type": "http",
  "dependencies": {
    "python": ">=3.10",
    "packages": ["requests", "beautifulsoup4"]
  },
  "config_schema": {
    "api_key": {
      "type": "string",
      "required": true,
      "encrypted": true,
      "description": "Google Search API Key"
    }
  },
  "http": {
    "port": 8080,
    "start_command": "python server.py",
    "health_check": "/health"
  },
  "cli": null
}
```

### 7.2 核心流程伪代码

```rust
// server/crates/skills/src/manager.rs

pub struct SkillsManager {
    skills_dir: PathBuf,              // ~/.dgos/skills/
    registry_url: String,             // 官方仓库 URL
    db: DatabasePool,
    http_clients: HashMap<String, HttpClient>,  // 已启动的 HTTP Skills
}

impl SkillsManager {
    // B02-01: 获取仓库 Skills 列表
    pub async fn fetch_registry(&self) -> Result<Vec<SkillInfo>> {
        // 从官方仓库获取
        let response = reqwest::get(&self.registry_url).await?;
        let registry: SkillRegistry = response.json().await?;
        
        // 缓存到数据库
        for skill in &registry.skills {
            self.db.upsert_registry_cache(skill).await?;
        }
        
        Ok(registry.skills)
    }
    
    // B02-02: 安装 Skill
    pub async fn install_skill(&mut self, skill_id: &str, user_id: &str) -> Result<Skill> {
        // 从仓库获取 Skill 信息
        let skill_info = self.db.get_registry_skill(skill_id).await?;
        
        // B02-03: 检查依赖
        self.check_dependencies(&skill_info.manifest)?;
        
        // 下载 Skill
        let download_url = &skill_info.download_url;
        let response = reqwest::get(download_url).await?;
        let bytes = response.bytes().await?;
        
        // 保存到临时目录
        let temp_path = format!("/tmp/dgos-skill-{}.tar.gz", skill_id);
        fs::write(&temp_path, &bytes).await?;
        
        // 解压到安装目录
        let install_path = self.skills_dir.join(skill_id);
        extract_tarball(&temp_path, &install_path)?;
        
        // 验证完整性
        verify_skill_integrity(&install_path)?;
        
        // 保存到数据库
        let skill = Skill {
            id: Uuid::new_v4().to_string(),
            skill_id: skill_id.to_string(),
            name: skill_info.name,
            version: skill_info.latest_version,
            install_path: install_path.to_string_lossy().to_string(),
            call_type: skill_info.manifest.call_type,
            enabled: true,
            config: None,
            manifest: skill_info.manifest,
            user_id: user_id.to_string(),
        };
        
        self.db.insert_skill(&skill).await?;
        
        // 清理临时文件
        fs::remove_file(&temp_path).await?;
        
        Ok(skill)
    }
    
    // B02-03: 检查依赖
    fn check_dependencies(&self, manifest: &SkillManifest) -> Result<()> {
        if let Some(python_req) = &manifest.dependencies.python {
            let python_version = get_python_version()?;
            if !version_satisfies(&python_version, python_req) {
                return Err(Error::DependencyNotSatisfied(
                    format!("需要 Python {}", python_req)
                ));
            }
        }
        
        if let Some(node_req) = &manifest.dependencies.node {
            let node_version = get_node_version()?;
            if !version_satisfies(&node_version, node_req) {
                return Err(Error::DependencyNotSatisfied(
                    format!("需要 Node.js {}", node_req)
                ));
            }
        }
        
        Ok(())
    }
    
    // B02-04: 调用 HTTP Skill
    pub async fn call_http_skill(
        &mut self,
        skill_id: &str,
        params: serde_json::Value
    ) -> Result<serde_json::Value> {
        // 获取 Skill 配置
        let skill = self.db.get_skill_by_skill_id(skill_id).await?;
        
        // B02-05: 检查是否启用
        if !skill.enabled {
            return Err(Error::SkillDisabled);
        }
        
        // B02-06: 检查配置是否完整
        if skill.manifest.config_schema.is_some() && skill.config.is_none() {
            return Err(Error::SkillNotConfigured);
        }
        
        // B02-07: 启动 HTTP 服务（如果未启动）
        if !self.http_clients.contains_key(skill_id) {
            self.start_http_skill(&skill).await?;
        }
        
        // 获取 HTTP 客户端
        let client = self.http_clients.get(skill_id)
            .ok_or(Error::SkillNotStarted)?;
        
        // B02-08: 发送 HTTP 请求
        let start_time = Instant::now();
        let response = client
            .post("/execute")
            .json(&params)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        
        let latency_ms = start_time.elapsed().as_millis() as i32;
        
        // B02-09: 解析结果
        let result: serde_json::Value = response.json().await?;
        
        // 记录使用日志
        self.log_usage(skill_id, &params, &result, true, latency_ms).await?;
        
        Ok(result)
    }
    
    // B02-10: 启动 HTTP Skill
    async fn start_http_skill(&mut self, skill: &Skill) -> Result<()> {
        let http_config = skill.manifest.http.as_ref()
            .ok_or(Error::NotHttpSkill)?;
        
        let cwd = PathBuf::from(&skill.install_path);
        let command = &http_config.start_command;
        
        // 启动进程
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .env("PORT", http_config.port.to_string())
            .env("CONFIG", serde_json::to_string(&skill.config)?)
            .spawn()?;
        
        // 等待服务启动（检查 health check）
        let base_url = format!("http://localhost:{}", http_config.port);
        let health_url = format!("{}{}", base_url, http_config.health_check);
        
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Ok(response) = reqwest::get(&health_url).await {
                if response.status().is_success() {
                    // 保存客户端
                    let client = HttpClient::new(&base_url);
                    self.http_clients.insert(skill.skill_id.clone(), client);
                    return Ok(());
                }
            }
        }
        
        // 超时，杀掉进程
        child.kill().await?;
        Err(Error::SkillStartTimeout)
    }
    
    // B02-11: 调用 CLI Skill
    pub async fn call_cli_skill(
        &self,
        skill_id: &str,
        params: serde_json::Value
    ) -> Result<serde_json::Value> {
        // 获取 Skill 配置
        let skill = self.db.get_skill_by_skill_id(skill_id).await?;
        
        // 检查是否启用
        if !skill.enabled {
            return Err(Error::SkillDisabled);
        }
        
        let cli_config = skill.manifest.cli.as_ref()
            .ok_or(Error::NotCliSkill)?;
        
        let cwd = PathBuf::from(&skill.install_path);
        let command = &cli_config.command;
        
        // B02-12: 执行命令
        let start_time = Instant::now();
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .env("INPUT", serde_json::to_string(&params)?)
            .env("CONFIG", serde_json::to_string(&skill.config)?)
            .output()
            .await?;
        
        let latency_ms = start_time.elapsed().as_millis() as i32;
        
        // B02-13: 解析输出
        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr).to_string();
            self.log_usage(skill_id, &params, &json!({"error": error_msg}), false, latency_ms).await?;
            return Err(Error::SkillExecutionFailed(error_msg));
        }
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value = serde_json::from_str(&stdout)?;
        
        // 记录使用日志
        self.log_usage(skill_id, &params, &result, true, latency_ms).await?;
        
        Ok(result)
    }
    
    // B02-14: 更新 Skill
    pub async fn update_skill(&mut self, skill_id: &str) -> Result<Skill> {
        // 获取当前 Skill
        let old_skill = self.db.get_skill_by_skill_id(skill_id).await?;
        
        // 获取最新版本信息
        let skill_info = self.db.get_registry_skill(skill_id).await?;
        
        // 检查版本
        if old_skill.version == skill_info.latest_version {
            return Ok(old_skill);
        }
        
        // B02-15: 备份旧版本
        let backup_path = format!("{}.backup", old_skill.install_path);
        fs::rename(&old_skill.install_path, &backup_path).await?;
        
        // 下载新版本
        let download_url = &skill_info.download_url;
        let response = reqwest::get(download_url).await?;
        let bytes = response.bytes().await?;
        
        // 保存到临时目录
        let temp_path = format!("/tmp/dgos-skill-{}-{}.tar.gz", skill_id, skill_info.latest_version);
        fs::write(&temp_path, &bytes).await?;
        
        // 解压
        let install_path = PathBuf::from(&old_skill.install_path);
        extract_tarball(&temp_path, &install_path)?;
        
        // 更新数据库
        self.db.update_skill_version(skill_id, &skill_info.latest_version).await?;
        
        // 清理
        fs::remove_file(&temp_path).await?;
        
        // 如果是 HTTP Skill，重启服务
        if old_skill.call_type == "http" {
            self.http_clients.remove(skill_id);
            // 下次调用时自动启动
        }
        
        Ok(self.db.get_skill_by_skill_id(skill_id).await?)
    }
}
```

### 7.3 分支到测试追踪表

| 分支 ID | 分支描述 | AC 映射 | E2E 映射 |
| --- | --- | --- | --- |
| B02-01 | 获取仓库列表 | AC01, AC02 | - |
| B02-02 | 安装 Skill | AC05, AC06 | V1-E2E-02 |
| B02-03 | 检查依赖 | AC04 | - |
| B02-04 | 调用 HTTP Skill | AC08 | V1-E2E-02 |
| B02-05 | 检查是否启用 | AC11 | - |
| B02-06 | 检查配置完整性 | AC07 | - |
| B02-07 | 启动 HTTP 服务 | AC08 | - |
| B02-08 | 发送 HTTP 请求 | AC08 | - |
| B02-09 | 解析结果 | AC08 | - |
| B02-10 | 启动 HTTP Skill | AC08 | - |
| B02-11 | 调用 CLI Skill | AC09 | V1-E2E-02 |
| B02-12 | 执行 CLI 命令 | AC09 | - |
| B02-13 | 解析 CLI 输出 | AC09 | - |
| B02-14 | 更新 Skill | AC14 | - |
| B02-15 | 备份旧版本 | AC14 | - |

## 8. 错误码设计

| 错误码 | 错误信息 | 场景 | 用户操作建议 |
| --- | --- | --- | --- |
| 1002001 | skill_not_found | Skill 不存在 | 检查 Skill ID |
| 1002002 | skill_already_installed | Skill 已安装 | 询问是否重新安装 |
| 1002003 | dependency_not_satisfied | 依赖不满足 | 安装依赖（Python/Node.js） |
| 1002004 | skill_disabled | Skill 已禁用 | 启用 Skill |
| 1002005 | skill_not_configured | Skill 未配置 | 配置 API Key 等 |
| 1002006 | skill_not_started | HTTP Skill 未启动 | 检查日志或重启 |
| 1002007 | skill_execution_failed | Skill 执行失败 | 查看错误信息 |
| 1002008 | skill_timeout | Skill 调用超时 | 检查 Skill 状态或网络 |
| 1002009 | skill_start_timeout | HTTP Skill 启动超时 | 检查依赖或端口占用 |
| 1002010 | registry_unreachable | 仓库不可达 | 检查网络或使用缓存 |

详见 [V1 错误码](../../../04-技术架构/当前版本/V1-错误码.md)（待创建）。

## 9. 非功能需求

### 9.1 性能需求

| 指标 | 目标 | 测量方式 |
| --- | --- | --- |
| 仓库列表获取 | < 3 秒 | 集成测试 |
| Skill 安装 | < 30 秒（取决于大小） | 集成测试 |
| HTTP Skill 启动 | < 5 秒 | 集成测试 |
| HTTP Skill 调用 | < 5 秒（取决于 Skill） | 集成测试 |
| CLI Skill 调用 | < 3 秒（取决于 Skill） | 集成测试 |
| 并发调用支持 | > 50 QPS | 压力测试 |

### 9.2 安全需求

| 需求 | 实现方式 |
| --- | --- |
| 配置加密存储 | 使用 AES-256 加密 API Key |
| Skills 完整性验证 | 下载后验证 checksum |
| 进程隔离 | Skills 独立进程运行 |
| 敏感信息脱敏 | 日志中 API Key 只显示前 4 位 |

### 9.3 可用性需求

| 需求 | 实现方式 |
| --- | --- |
| 离线使用 | 缓存仓库数据，离线可用 |
| 失败恢复 | Skill 执行失败自动清理 |
| 版本回滚 | 更新失败可回滚到旧版本 |

## 10. 实现与验证

### 10.1 后端测试

**目标资产**：
- `server/crates/skills/src/manager.rs`
- `server/crates/skills/src/registry.rs`
- `server/crates/skills/src/caller.rs`
- `server/crates/skills/tests/registry.rs`
- `server/crates/skills/tests/installation.rs`
- `server/crates/skills/tests/skill_call.rs`
- `server/crates/skills/tests/dependency.rs`
- `server/crates/skills/tests/update.rs`

**测试命令**：
```bash
cd server && cargo test -p dgos-skills
```

**验证记录**（待开发后填写）：
```
日期：YYYY-MM-DD
执行人：XXX
环境：本地开发环境

结果：
- registry.rs：0/3 通过（待实现）
- installation.rs：0/4 通过（待实现）
- skill_call.rs：0/3 通过（待实现）
- dependency.rs：0/1 通过（待实现）
- update.rs：0/2 通过（待实现）

详细覆盖说明：
- AC01～AC15：待实现
```

### 10.2 前端集成测试

**目标资产**：
- `frontend/apps/desktop/src/hooks/useSkills.ts`
- `frontend/apps/desktop/src/views/SkillsSettings.tsx`
- `frontend/apps/desktop/src/__tests__/skills_settings.test.tsx`

**测试命令**：
```bash
cd frontend && pnpm test -- skills_settings
```

**验证记录**（待开发后填写）：
```
日期：YYYY-MM-DD
执行人：XXX
环境：本地开发环境

结果：
- skills_settings.test.tsx：0/3 通过（待实现）

详细覆盖说明：
- 界面交互测试：待实现
```

### 10.3 E2E 测试

**E2E 场景**：V1-E2E-02（完整的 Skill 安装和调用流程）

**测试资产**：
- `tests/e2e/skills_workflow.spec.ts`

**测试命令**：
```bash
pnpm test:e2e -- skills_workflow
```

**验证记录**（待开发后填写）：
```
日期：YYYY-MM-DD
执行人：XXX
环境：集成测试环境

结果：
- skills_workflow.spec.ts：0/1 通过（待实现）

详细说明：
- 完整流程测试：待实现
```

## 11. 风险与依赖

### 11.1 技术风险

| 风险 | 概率 | 影响 | 缓解措施 |
| --- | --- | --- | --- |
| Skills 生态不成熟 | 中 | 中 | 自己开发几个官方 Skills |
| HTTP Skills 启动失败 | 中 | 中 | 完善错误提示，提供调试日志 |
| CLI Skills 进程管理 | 中 | 中 | 使用成熟的进程管理库 |
| 依赖版本不兼容 | 低 | 中 | 明确版本要求，提供安装指南 |

### 11.2 外部依赖

| 依赖 | 类型 | 风险 | 应对 |
| --- | --- | --- | --- |
| 官方 Skills 仓库 | 外部服务 | 可用性依赖 | 缓存数据，离线可用 |
| Python/Node.js | 运行时 | 用户环境依赖 | 提供安装指南 |
| Skills 自身 | 第三方代码 | 质量依赖 | 人工审核，评分系统 |

### 11.3 功能依赖

| 依赖功能 | 状态 | 影响 |
| --- | --- | --- |
| 本地用户管理（V1-FR-008） | 规划中 | 需要先实现用户系统 |
| SQLite 数据库 | 待初始化 | 需要先创建表结构 |
| Provider 框架（V1-FR-001） | 规划中 | Skills 可能调用 Provider |

## 12. 实现状态

| 模块 | 状态 | 进度 | 负责人 | 备注 |
| --- | --- | --- | --- | --- |
| Skills Manager | 🔴 未开始 | 0% | TBD | - |
| 仓库连接 | 🔴 未开始 | 0% | TBD | - |
| Skills 安装 | 🔴 未开始 | 0% | TBD | - |
| HTTP Caller | 🔴 未开始 | 0% | TBD | - |
| CLI Caller | 🔴 未开始 | 0% | TBD | - |
| 依赖检查 | 🔴 未开始 | 0% | TBD | - |
| 更新机制 | 🔴 未开始 | 0% | TBD | - |
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
