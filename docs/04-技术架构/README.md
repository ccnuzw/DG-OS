# 技术架构导航

## 系统级

- [总体架构](总体架构.md)
- [服务与领域边界](服务与领域边界.md)
- [技术栈选型](技术栈选型.md)

## V1 当前架构

- [V1 总体架构](当前版本/V1-总体架构.md)
- [V1 接口契约](当前版本/V1-接口契约.md)
- [V1 OpenAPI](当前版本/V1-openapi.yaml)
- [V1 数据模型](当前版本/V1-数据模型.md)
- [V1 统一设计系统](当前版本/V1-统一设计系统.md)
- [V1 模块系统设计](当前版本/V1-模块系统设计.md)

## 系统设计

- [Provider 适配框架设计](system-design/Provider适配框架设计.md)
- [Skills 管理系统设计](system-design/Skills管理系统设计.md)
- [MCP 管理系统设计](system-design/MCP管理系统设计.md)
- [模块加载与热更新设计](system-design/模块加载与热更新设计.md)
- [权限系统设计](system-design/权限系统设计.md)
- [局域网协同设计](system-design/局域网协同设计.md)

## 权威边界

- HTTP 字段级权威只有 OpenAPI；Markdown 契约负责公共语义（鉴权、错误包体），功能文档引用 operationId 并记录业务语义，不复制 schema。
- 数据物理事实以 migration 为准；数据模型文档负责解释约束、状态与演进。
- 设计系统规范为所有模块强制遵守，不得自定义样式覆盖。
