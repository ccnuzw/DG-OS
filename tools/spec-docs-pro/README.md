# spec-docs-pro（文档检查与离线浏览器生成器）

`docs/` 的一致性检查和离线产品文档浏览器由这里的脚本生成。内容是 spec-docs-pro 2.0.0 的 `scripts/` 与 `assets/browser/` 拷贝（`package.json` 声明 MIT），仅 `viewer.css` 有一处样式修复（见下文“与上游的差异”）；项目配置在 [`docs/spec-docs.config.json`](../../docs/spec-docs.config.json)。

| 命令 | 作用 |
| --- | --- |
| `npm run docs:check` | 只读检查编号、链接与锚点、AC 与测试设计、分支追踪、状态词、演进矩阵、规划版本文档。有错误时退出码为 1 |
| `npm run docs:browser` | 重新生成 `docs/90-参考资料/产品文档浏览器.html` 与 `产品文档浏览器-文档数据.js` |
| `npm run docs:browser:check` | 在临时目录重新生成，与已提交文件比较；文档变化后忘记重新生成时失败 |

规则说明见 skill 的 `references/check-rules.md`。升级时整体替换 `scripts/` 与 `assets/browser/`，再运行上面三条命令；项目专属行为只放在 `docs/spec-docs.config.json`，不改这里的脚本。

## 与上游的差异

只有一处，位于 `assets/browser/viewer.css`：上游把表格样式（`border-collapse`、单元格内边距、表头底色）只写给 `.article table`，而“功能”和“演进”视图的表格在 `.feature-table` 与 `.evolution-grid` 里，不在 `.article` 内，因此渲染成无内边距、无边框的裸表格。这里把这两个选择器并入同一组规则，并让功能清单的编号、状态等列不换行。升级时若上游已修复，丢弃这处改动；否则重新应用并运行 `npm test -- --run tests/doc-browser.test.tsx`，其中 `table styling` 检查会在样式丢失时失败。

生成数据里的 `generatedAt` 是生成时间，`docs:browser:check` 比较时忽略这一项。
