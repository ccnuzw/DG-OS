# 参考资料索引

本目录存放外部参考、调研材料、对话记录和辅助文档。参考资料不是权威文档，仅供查阅。

## 对话记录

- [初始讨论完整记录](2024-01-XX-初始讨论完整记录.md)：DGOS 从概念到架构设计的对话记录，含老项目 infinite-canvas-ccnuzw 的问题分析（第一部分）。文件名中的 `2024-01-XX` 是原始占位日期，该文件于 2026-10-26 首次提交；为避免链接失效，暂不改名。

## 文档浏览器（生成产物）

- [产品文档浏览器](产品文档浏览器.html)：离线浏览 `docs/` 的单页应用，数据来自同目录的 `产品文档浏览器-文档数据.js`，两个文件需放在一起
- [文档浏览器生成报告](文档浏览器生成报告.md)：生成命令、数据来源和验证记录

重新生成用 `npm run docs:browser`，检查用 `npm run docs:check` 与 `npm run docs:browser:check`。浏览器文件是派生结果，不手工编辑；内容与 Markdown 不一致时以 Markdown 为准。

## 待补充（尚未创建）

以下材料曾在本页列出，但文件从未创建：

- 对话总结与待办事项
- 老项目教训分析（目前可参考初始讨论记录第一部分）
- 技术选型调研：Rust vs Node.js、SQLite vs PostgreSQL、Module Federation、Skills 标准、MCP 协议

技术选型的结论已记录在[技术栈选型](../04-技术架构/技术栈选型.md)和 [V1 冻结决策](../06-决策记录/V1-冻结决策.md)。

## 外部参考

- [Model Context Protocol](https://modelcontextprotocol.io/)
- [Tauri 官方文档](https://tauri.app/)
- [Module Federation 文档](https://module-federation.io/)

## 使用说明

- 决策依据见 [V1 冻结决策](../06-决策记录/V1-冻结决策.md)
- 技术方案见 [技术架构](../04-技术架构/README.md)
