# spec-docs-pro（文档检查与离线浏览器生成器）

`docs/` 的一致性检查和离线产品文档浏览器由这里的脚本生成。内容是 spec-docs-pro 2.0.0 的 `scripts/` 与 `assets/browser/` 原样拷贝（`package.json` 声明 MIT），没有项目专属改动；项目配置在 [`docs/spec-docs.config.json`](../../docs/spec-docs.config.json)。

| 命令 | 作用 |
| --- | --- |
| `npm run docs:check` | 只读检查编号、链接与锚点、AC 与测试设计、分支追踪、状态词、演进矩阵、规划版本文档。有错误时退出码为 1 |
| `npm run docs:browser` | 重新生成 `docs/90-参考资料/产品文档浏览器.html` 与 `产品文档浏览器-文档数据.js` |
| `npm run docs:browser:check` | 在临时目录重新生成，与已提交文件比较；文档变化后忘记重新生成时失败 |

规则说明见 skill 的 `references/check-rules.md`。升级时整体替换 `scripts/` 与 `assets/browser/`，再运行上面三条命令；项目专属行为只放在 `docs/spec-docs.config.json`，不改这里的脚本。

生成数据里的 `generatedAt` 是生成时间，`docs:browser:check` 比较时忽略这一项。
