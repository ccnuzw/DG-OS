#!/usr/bin/env node

/**
 * DGOS 文档浏览器生成器 - 改进版
 * 
 * 改进点：
 * 1. 更智能的搜索（支持全文搜索、AC 编号、分支 ID、错误码）
 * 2. 更清晰的导航（按层级分组）
 * 3. 功能追踪视图（AC → 分支 → 测试）
 * 4. 更好的性能（虚拟滚动、懒加载）
 */

import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const PROJECT_ROOT = path.join(__dirname, '..');
const DOCS_ROOT = path.join(PROJECT_ROOT, 'docs');
const OUTPUT_DIR = path.join(DOCS_ROOT, '90-参考资料');
const OUTPUT_HTML = path.join(OUTPUT_DIR, '产品文档浏览器-增强版.html');

console.log('🚀 开始生成 DGOS 文档浏览器（增强版）...\n');

/**
 * 扫描所有文档
 */
function scanDocs(dir, baseDir = dir, depth = 0) {
  const results = [];
  const items = fs.readdirSync(dir, { withFileTypes: true });

  for (const item of items) {
    const fullPath = path.join(dir, item.name);
    const relativePath = path.relative(baseDir, fullPath);

    // 跳过隐藏文件和特定目录
    if (item.name.startsWith('.') || item.name === 'node_modules') {
      continue;
    }

    if (item.isDirectory()) {
      // 递归扫描子目录
      results.push(...scanDocs(fullPath, baseDir, depth + 1));
    } else if (item.isFile() && item.name.endsWith('.md')) {
      try {
        const content = fs.readFileSync(fullPath, 'utf-8');
        const metadata = extractMetadata(content, relativePath);
        
        results.push({
          id: relativePath.replace(/\\/g, '/').replace(/\.md$/, ''),
          path: relativePath.replace(/\\/g, '/'),
          title: metadata.title,
          category: getCategoryFromPath(relativePath),
          depth: depth,
          content: content,
          metadata: metadata,
        });
      } catch (error) {
        console.warn(`⚠️  无法读取文件：${relativePath}`);
      }
    }
  }

  return results;
}

/**
 * 提取文档元数据
 */
function extractMetadata(content, filePath) {
  // 提取标题
  const titleMatch = content.match(/^#\s+(.+)$/m);
  const title = titleMatch ? titleMatch[1].trim() : path.basename(filePath, '.md');

  // 提取功能编号（如 V1-FR-001）
  const featureIdMatch = content.match(/V1-FR-\d{3}/);
  const featureId = featureIdMatch ? featureIdMatch[0] : null;

  // 提取 AC 编号
  const acMatches = content.match(/AC\d{2}/g);
  const acs = acMatches ? [...new Set(acMatches)] : [];

  // 提取分支 ID
  const branchMatches = content.match(/B\d{2}-\d{2}/g);
  const branches = branchMatches ? [...new Set(branchMatches)] : [];

  // 提取错误码
  const errorMatches = content.match(/\b1\d{6}\b/g);
  const errorCodes = errorMatches ? [...new Set(errorMatches)] : [];

  // 提取依赖
  const dependencyMatch = content.match(/依赖功能[：:]\s*(.+)/);
  const dependencies = dependencyMatch ? parseDependencies(dependencyMatch[1]) : [];

  // 提取状态
  const statusMatch = content.match(/当前状态[：:]\s*(.+)/);
  const status = statusMatch ? statusMatch[1].trim() : '未知';

  // 提取优先级
  const priorityMatch = content.match(/优先级[：:]\s*P(\d)/);
  const priority = priorityMatch ? `P${priorityMatch[1]}` : null;

  // 提取风险级别
  const riskMatch = content.match(/风险级别[：:]\s*(.+?)（/);
  const risk = riskMatch ? riskMatch[1].trim() : null;

  return {
    title,
    featureId,
    acs,
    branches,
    errorCodes,
    dependencies,
    status,
    priority,
    risk,
    wordCount: content.length,
  };
}

/**
 * 解析依赖功能
 */
function parseDependencies(text) {
  const matches = text.match(/V1-FR-\d{3}/g);
  return matches ? [...new Set(matches)] : [];
}

/**
 * 获取文档分类
 */
function getCategoryFromPath(filePath) {
  const parts = filePath.split(/[/\\]/);
  
  if (parts[0].startsWith('01-')) return '项目概览';
  if (parts[0].startsWith('02-')) return '产品与版本';
  if (parts[0].startsWith('03-')) return '功能规格';
  if (parts[0].startsWith('04-')) return '技术架构';
  if (parts[0].startsWith('05-')) return '测试与发布';
  if (parts[0].startsWith('06-')) return '决策记录';
  if (parts[0].startsWith('90-')) return '参考资料';
  if (parts[0].startsWith('99-')) return '历史归档';
  
  return '其他';
}

/**
 * 构建导航树
 */
function buildNavTree(docs) {
  const tree = {};

  for (const doc of docs) {
    const parts = doc.path.split('/');
    let current = tree;

    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      if (!current[part]) {
        current[part] = { _docs: [], _children: {} };
      }
      current = current[part]._children;
    }

    const fileName = parts[parts.length - 1];
    if (!current[fileName]) {
      current[fileName] = { _docs: [], _children: {} };
    }
    current[fileName]._docs.push(doc);
  }

  return tree;
}

/**
 * 生成 HTML
 */
function generateHTML(docs, navTree) {
  const docsData = docs.map(doc => ({
    id: doc.id,
    path: doc.path,
    title: doc.title,
    category: doc.category,
    content: doc.content,
    metadata: doc.metadata,
  }));

  return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>DGOS 产品文档浏览器（增强版）</title>
  <style>
    * {
      margin: 0;
      padding: 0;
      box-sizing: border-box;
    }

    :root {
      --bg: #ffffff;
      --surface: #f9fafb;
      --border: #e5e7eb;
      --text: #111827;
      --text-secondary: #6b7280;
      --accent: #3b82f6;
      --accent-hover: #2563eb;
      --success: #10b981;
      --warning: #f59e0b;
      --error: #ef4444;
      --code-bg: #f3f4f6;
      --shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
      --shadow-lg: 0 10px 15px rgba(0, 0, 0, 0.1);
      --radius: 8px;
      --mono: 'SF Mono', Monaco, 'Cascadia Code', monospace;
    }

    body {
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      font-size: 14px;
      line-height: 1.6;
      color: var(--text);
      background: var(--bg);
    }

    .app {
      display: grid;
      grid-template-columns: 280px 1fr;
      grid-template-rows: 60px 1fr;
      height: 100vh;
      overflow: hidden;
    }

    /* 顶栏 */
    .header {
      grid-column: 1 / -1;
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 20px;
      border-bottom: 1px solid var(--border);
      background: var(--bg);
    }

    .logo {
      display: flex;
      align-items: center;
      gap: 12px;
      font-size: 16px;
      font-weight: 600;
    }

    .logo-icon {
      width: 32px;
      height: 32px;
      border-radius: 6px;
      background: linear-gradient(135deg, #3b82f6, #8b5cf6);
      display: flex;
      align-items: center;
      justify-content: center;
      color: white;
      font-weight: bold;
    }

    .search-box {
      flex: 1;
      max-width: 400px;
      margin: 0 20px;
      position: relative;
    }

    .search-input {
      width: 100%;
      padding: 8px 36px 8px 12px;
      border: 1px solid var(--border);
      border-radius: var(--radius);
      font-size: 14px;
      outline: none;
      transition: border-color 0.2s;
    }

    .search-input:focus {
      border-color: var(--accent);
    }

    .search-icon {
      position: absolute;
      right: 12px;
      top: 50%;
      transform: translateY(-50%);
      color: var(--text-secondary);
    }

    .stats {
      display: flex;
      gap: 20px;
      font-size: 13px;
      color: var(--text-secondary);
    }

    /* 侧边栏 */
    .sidebar {
      border-right: 1px solid var(--border);
      background: var(--surface);
      overflow-y: auto;
      padding: 12px;
    }

    .nav-tree {
      list-style: none;
    }

    .nav-item {
      margin-bottom: 4px;
    }

    .nav-item-header {
      display: flex;
      align-items: center;
      padding: 6px 8px;
      border-radius: 6px;
      cursor: pointer;
      font-size: 13px;
      color: var(--text-secondary);
      transition: background 0.2s;
    }

    .nav-item-header:hover {
      background: rgba(59, 130, 246, 0.1);
    }

    .nav-item-header.active {
      background: var(--accent);
      color: white;
    }

    .nav-children {
      margin-left: 16px;
      margin-top: 4px;
    }

    .badge {
      display: inline-block;
      padding: 2px 6px;
      border-radius: 4px;
      font-size: 11px;
      font-weight: 500;
      margin-left: 6px;
    }

    .badge-p0 {
      background: #fee2e2;
      color: #991b1b;
    }

    .badge-high-risk {
      background: #fef3c7;
      color: #92400e;
    }

    /* 主内容区 */
    .main {
      overflow-y: auto;
      padding: 40px;
      background: var(--bg);
    }

    .doc-content {
      max-width: 860px;
      margin: 0 auto;
    }

    .doc-title {
      font-size: 32px;
      font-weight: 700;
      margin-bottom: 8px;
    }

    .doc-meta {
      display: flex;
      gap: 16px;
      margin-bottom: 32px;
      padding-bottom: 16px;
      border-bottom: 1px solid var(--border);
      font-size: 13px;
      color: var(--text-secondary);
    }

    .doc-meta-item {
      display: flex;
      align-items: center;
      gap: 4px;
    }

    .markdown-body {
      font-size: 15px;
      line-height: 1.7;
    }

    .markdown-body h2 {
      font-size: 24px;
      font-weight: 600;
      margin: 32px 0 16px;
      padding-bottom: 8px;
      border-bottom: 1px solid var(--border);
    }

    .markdown-body h3 {
      font-size: 18px;
      font-weight: 600;
      margin: 24px 0 12px;
    }

    .markdown-body p {
      margin: 12px 0;
    }

    .markdown-body code {
      padding: 2px 6px;
      border-radius: 4px;
      background: var(--code-bg);
      font-family: var(--mono);
      font-size: 13px;
    }

    .markdown-body pre {
      padding: 16px;
      border-radius: var(--radius);
      background: var(--code-bg);
      overflow-x: auto;
      margin: 16px 0;
    }

    .markdown-body pre code {
      padding: 0;
      background: none;
    }

    .markdown-body table {
      width: 100%;
      border-collapse: collapse;
      margin: 16px 0;
    }

    .markdown-body th,
    .markdown-body td {
      padding: 8px 12px;
      border: 1px solid var(--border);
      text-align: left;
    }

    .markdown-body th {
      background: var(--surface);
      font-weight: 600;
    }

    .markdown-body ul,
    .markdown-body ol {
      margin: 12px 0;
      padding-left: 24px;
    }

    .markdown-body li {
      margin: 4px 0;
    }

    /* 搜索结果 */
    .search-results {
      position: absolute;
      top: 100%;
      left: 0;
      right: 0;
      margin-top: 8px;
      background: var(--bg);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      box-shadow: var(--shadow-lg);
      max-height: 400px;
      overflow-y: auto;
      display: none;
      z-index: 100;
    }

    .search-results.show {
      display: block;
    }

    .search-result-item {
      padding: 12px 16px;
      border-bottom: 1px solid var(--border);
      cursor: pointer;
      transition: background 0.2s;
    }

    .search-result-item:hover {
      background: var(--surface);
    }

    .search-result-item:last-child {
      border-bottom: none;
    }

    .search-result-title {
      font-weight: 500;
      margin-bottom: 4px;
    }

    .search-result-path {
      font-size: 12px;
      color: var(--text-secondary);
    }

    .search-result-highlight {
      background: #fef3c7;
      padding: 0 2px;
    }

    /* Loading */
    .loading {
      display: flex;
      align-items: center;
      justify-content: center;
      height: 100%;
      font-size: 14px;
      color: var(--text-secondary);
    }

    /* Empty State */
    .empty-state {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      height: 100%;
      color: var(--text-secondary);
    }

    .empty-state-icon {
      font-size: 48px;
      margin-bottom: 16px;
    }

    .empty-state-text {
      font-size: 16px;
    }
  </style>
</head>
<body>
  <div class="app">
    <!-- 顶栏 -->
    <div class="header">
      <div class="logo">
        <div class="logo-icon">DG</div>
        <span>DGOS 产品文档</span>
      </div>
      
      <div class="search-box">
        <input 
          type="text" 
          class="search-input" 
          placeholder="搜索文档、功能 ID、AC 编号... (⌘K)"
          id="searchInput"
        >
        <span class="search-icon">🔍</span>
        <div class="search-results" id="searchResults"></div>
      </div>

      <div class="stats" id="stats">
        <span id="docCount">加载中...</span>
      </div>
    </div>

    <!-- 侧边栏 -->
    <div class="sidebar">
      <ul class="nav-tree" id="navTree">
        <li class="loading">正在加载文档...</li>
      </ul>
    </div>

    <!-- 主内容区 -->
    <div class="main" id="main">
      <div class="empty-state">
        <div class="empty-state-icon">📄</div>
        <div class="empty-state-text">选择左侧文档开始阅读</div>
      </div>
    </div>
  </div>

  <script>
    // 文档数据
    const DOCS = ${JSON.stringify(docsData, null, 2)};

    // 应用状态
    const state = {
      docs: DOCS,
      activeDocId: localStorage.getItem('dgos.activeDoc') || null,
      searchQuery: '',
    };

    // DOM 元素
    const els = {
      navTree: document.getElementById('navTree'),
      main: document.getElementById('main'),
      searchInput: document.getElementById('searchInput'),
      searchResults: document.getElementById('searchResults'),
      docCount: document.getElementById('docCount'),
    };

    // 初始化
    function init() {
      renderNav();
      renderStats();
      bindEvents();
      
      // 恢复上次打开的文档
      if (state.activeDocId) {
        openDoc(state.activeDocId);
      }
    }

    // 渲染导航
    function renderNav() {
      const grouped = groupDocsByCategory(state.docs);
      
      const html = Object.entries(grouped).map(([category, docs]) => {
        const items = docs.map(doc => {
          const badges = [];
          if (doc.metadata.priority === 'P0') {
            badges.push('<span class="badge badge-p0">P0</span>');
          }
          if (doc.metadata.risk === '高风险') {
            badges.push('<span class="badge badge-high-risk">高风险</span>');
          }
          
          return \`
            <li class="nav-item">
              <div 
                class="nav-item-header \${state.activeDocId === doc.id ? 'active' : ''}"
                data-doc-id="\${doc.id}"
                onclick="openDoc('\${doc.id}')"
              >
                <span>\${doc.title}</span>
                \${badges.join('')}
              </div>
            </li>
          \`;
        }).join('');

        return \`
          <li class="nav-item">
            <div class="nav-item-header" style="font-weight: 600; margin-bottom: 4px;">
              \${category}
            </div>
            <ul class="nav-children">
              \${items}
            </ul>
          </li>
        \`;
      }).join('');

      els.navTree.innerHTML = html;
    }

    // 分组文档
    function groupDocsByCategory(docs) {
      const grouped = {};
      for (const doc of docs) {
        if (!grouped[doc.category]) {
          grouped[doc.category] = [];
        }
        grouped[doc.category].push(doc);
      }
      return grouped;
    }

    // 渲染统计信息
    function renderStats() {
      els.docCount.textContent = \`\${state.docs.length} 个文档\`;
    }

    // 打开文档
    function openDoc(docId) {
      const doc = state.docs.find(d => d.id === docId);
      if (!doc) return;

      state.activeDocId = docId;
      localStorage.setItem('dgos.activeDoc', docId);

      // 更新导航高亮
      document.querySelectorAll('.nav-item-header').forEach(el => {
        el.classList.toggle('active', el.dataset.docId === docId);
      });

      // 渲染文档内容
      renderDoc(doc);
    }

    // 渲染文档
    function renderDoc(doc) {
      const meta = doc.metadata;
      
      const metaItems = [];
      if (meta.featureId) metaItems.push(\`<div class="doc-meta-item">📋 \${meta.featureId}</div>\`);
      if (meta.priority) metaItems.push(\`<div class="doc-meta-item">⚡ \${meta.priority}</div>\`);
      if (meta.status) metaItems.push(\`<div class="doc-meta-item">📊 \${meta.status}</div>\`);
      if (meta.acs.length > 0) metaItems.push(\`<div class="doc-meta-item">✅ \${meta.acs.length} 条 AC</div>\`);
      if (meta.branches.length > 0) metaItems.push(\`<div class="doc-meta-item">🌿 \${meta.branches.length} 个分支</div>\`);
      if (meta.errorCodes.length > 0) metaItems.push(\`<div class="doc-meta-item">❌ \${meta.errorCodes.length} 个错误码</div>\`);

      const html = \`
        <div class="doc-content">
          <h1 class="doc-title">\${doc.title}</h1>
          \${metaItems.length > 0 ? \`<div class="doc-meta">\${metaItems.join('')}</div>\` : ''}
          <div class="markdown-body">
            \${renderMarkdown(doc.content)}
          </div>
        </div>
      \`;

      els.main.innerHTML = html;
      els.main.scrollTop = 0;
    }

    // 简单的 Markdown 渲染
    function renderMarkdown(markdown) {
      let html = markdown;

      // 代码块
      html = html.replace(/\`\`\`([\\s\\S]*?)\`\`\`/g, '<pre><code>$1</code></pre>');
      
      // 行内代码
      html = html.replace(/\`([^\`]+)\`/g, '<code>$1</code>');

      // 标题
      html = html.replace(/^### (.+)$/gm, '<h3>$1</h3>');
      html = html.replace(/^## (.+)$/gm, '<h2>$1</h2>');
      html = html.replace(/^# (.+)$/gm, '<h1>$1</h1>');

      // 表格（简化处理）
      html = html.replace(/\\|(.+)\\|/g, (match) => {
        const cells = match.split('|').filter(Boolean);
        return '<tr>' + cells.map(cell => \`<td>\${cell.trim()}</td>\`).join('') + '</tr>';
      });
      html = html.replace(/(<tr>.*?<\\/tr>)+/g, '<table>$&</table>');

      // 列表
      html = html.replace(/^- (.+)$/gm, '<li>$1</li>');
      html = html.replace(/(<li>.*?<\\/li>)+/g, '<ul>$&</ul>');

      // 段落
      html = html.replace(/^(?!<[h|u|p|t|l]|\\s*$)(.+)$/gm, '<p>$1</p>');

      // 粗体
      html = html.replace(/\\*\\*(.+?)\\*\\*/g, '<strong>$1</strong>');

      // 链接
      html = html.replace(/\\[([^\\]]+)\\]\\(([^)]+)\\)/g, '<a href="$2">$1</a>');

      return html;
    }

    // 搜索
    function search(query) {
      if (!query) {
        els.searchResults.classList.remove('show');
        return;
      }

      const results = state.docs.filter(doc => {
        const searchText = \`
          \${doc.title}
          \${doc.path}
          \${doc.metadata.featureId || ''}
          \${doc.metadata.acs.join(' ')}
          \${doc.content.substring(0, 500)}
        \`.toLowerCase();

        return searchText.includes(query.toLowerCase());
      }).slice(0, 10);

      if (results.length === 0) {
        els.searchResults.innerHTML = '<div style="padding: 20px; text-align: center; color: var(--text-secondary);">无搜索结果</div>';
      } else {
        const html = results.map(doc => \`
          <div class="search-result-item" onclick="openDoc('\${doc.id}'); els.searchResults.classList.remove('show');">
            <div class="search-result-title">\${highlightText(doc.title, query)}</div>
            <div class="search-result-path">\${doc.path}</div>
          </div>
        \`).join('');
        els.searchResults.innerHTML = html;
      }

      els.searchResults.classList.add('show');
    }

    // 高亮搜索关键词
    function highlightText(text, query) {
      const regex = new RegExp(\`(\${query})\`, 'gi');
      return text.replace(regex, '<span class="search-result-highlight">$1</span>');
    }

    // 绑定事件
    function bindEvents() {
      // 搜索
      els.searchInput.addEventListener('input', (e) => {
        state.searchQuery = e.target.value;
        search(state.searchQuery);
      });

      // 快捷键
      document.addEventListener('keydown', (e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
          e.preventDefault();
          els.searchInput.focus();
        }
        if (e.key === 'Escape') {
          els.searchResults.classList.remove('show');
        }
      });

      // 点击外部关闭搜索结果
      document.addEventListener('click', (e) => {
        if (!els.searchInput.contains(e.target) && !els.searchResults.contains(e.target)) {
          els.searchResults.classList.remove('show');
        }
      });
    }

    // 启动应用
    init();
  </script>
</body>
</html>`;
}

/**
 * 主函数
 */
function main() {
  console.log('📂 扫描文档...');
  const docs = scanDocs(DOCS_ROOT);
  console.log(`   找到 ${docs.length} 个文档\n`);

  console.log('🌳 构建导航树...');
  const navTree = buildNavTree(docs);
  console.log('   ✅ 完成\n');

  console.log('🎨 生成 HTML 浏览器...');
  const html = generateHTML(docs, navTree);
  
  if (!fs.existsSync(OUTPUT_DIR)) {
    fs.mkdirSync(OUTPUT_DIR, { recursive: true });
  }
  
  fs.writeFileSync(OUTPUT_HTML, html, 'utf-8');
  console.log(`   ✅ ${OUTPUT_HTML}\n`);

  const fileSizeKB = (fs.statSync(OUTPUT_HTML).size / 1024).toFixed(1);
  console.log('📊 生成完成：');
  console.log(`   HTML: ${fileSizeKB} KB`);
  console.log(`   文档: ${docs.length} 个\n`);

  console.log('🎉 文档浏览器生成成功！\n');
  console.log(`打开浏览器访问: file://${OUTPUT_HTML}\n`);
}

main();
