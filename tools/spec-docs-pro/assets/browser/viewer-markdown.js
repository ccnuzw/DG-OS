// ---------- markdown rendering (escape first, then add a small whitelist of markup) ----------

function stripFrontmatter(source) {
  return String(source || "").replace(/^\uFEFF?---\r?\n[\s\S]*?\r?\n---[ \t]*(\r?\n|$)/, "");
}

function renderMarkdown(source, sourcePath) {
  const lines = String(source || "").replace(/\r\n/g, "\n").split("\n");
  const html = [];
  const outline = [];
  const used = new Map();
  let index = 0;

  const headingId = text => {
    const base = slug(text) || "section";
    const count = used.get(base) || 0;
    used.set(base, count + 1);
    return { id: `h-${count ? `${base}-${count}` : base}`, slug: base };
  };

  while (index < lines.length) {
    const line = lines[index];

    const fence = /^\s*(`{3,}|~{3,})\s*([\w+-]*)/.exec(line);
    if (fence) {
      const marker = fence[1];
      const code = [];
      index += 1;
      while (index < lines.length && !new RegExp(`^\\s*${marker[0] === "`" ? "`" : "~"}{${marker.length},}\\s*$`).test(lines[index])) {
        code.push(lines[index]);
        index += 1;
      }
      index += 1;
      html.push(`<pre><code data-lang="${escapeHtml(fence[2])}">${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }

    const heading = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (heading) {
      const level = Math.min(heading[1].length, 4);
      const text = heading[2].replace(/[`*_]/g, "");
      const { id, slug: key } = headingId(text);
      outline.push({ id, text, level });
      html.push(`<h${level} id="${id}" data-slug="${escapeHtml(key)}">${inline(heading[2], sourcePath)}</h${level}>`);
      index += 1;
      continue;
    }

    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      html.push("<hr>");
      index += 1;
      continue;
    }

    if (isTableStartLine(lines, index)) {
      const block = [];
      while (index < lines.length && /^\s*\|.*\|\s*$/.test(lines[index])) {
        block.push(lines[index]);
        index += 1;
      }
      html.push(renderTable(block, sourcePath));
      continue;
    }

    if (/^\s*([-*+]|\d+[.)])\s+/.test(line)) {
      const block = [];
      while (index < lines.length && (/^\s*([-*+]|\d+[.)])\s+/.test(lines[index]) || (/^\s{2,}\S/.test(lines[index]) && block.length))) {
        block.push(lines[index]);
        index += 1;
      }
      html.push(renderList(block, sourcePath));
      continue;
    }

    if (/^\s*>/.test(line)) {
      const quote = [];
      while (index < lines.length && /^\s*>/.test(lines[index])) {
        quote.push(lines[index].replace(/^\s*>\s?/, ""));
        index += 1;
      }
      html.push(`<blockquote>${renderMarkdown(quote.join("\n"), sourcePath).html}</blockquote>`);
      continue;
    }

    const image = /^\s*!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)\s*$/.exec(line);
    if (image) {
      html.push(renderFigure(image[1], image[2], sourcePath));
      index += 1;
      continue;
    }

    if (!line.trim()) {
      index += 1;
      continue;
    }

    const paragraph = [line];
    index += 1;
    while (index < lines.length && lines[index].trim() && !/^\s*(#{1,6}\s|`{3,}|~{3,}|>|([-*+]|\d+[.)])\s)/.test(lines[index]) && !isTableStartLine(lines, index)) {
      paragraph.push(lines[index]);
      index += 1;
    }
    html.push(`<p>${inline(paragraph.join("\n"), sourcePath).replace(/\n/g, " ")}</p>`);
  }

  return { html: html.join("\n"), outline };
}

function renderList(block, sourcePath) {
  const items = [];
  for (const raw of block) {
    const match = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(raw);
    if (match) items.push({ indent: match[1].replace(/\t/g, "  ").length, ordered: /\d/.test(match[2]), text: match[3] });
    else if (items.length) items[items.length - 1].text += ` ${raw.trim()}`;
  }
  const build = (start, indent) => {
    const ordered = items[start].ordered;
    const out = [];
    let i = start;
    while (i < items.length && items[i].indent >= indent) {
      if (items[i].indent > indent) {
        const nested = build(i, items[i].indent);
        out[out.length - 1] = out[out.length - 1].replace(/<\/li>$/, `${nested.html}</li>`);
        i = nested.next;
        continue;
      }
      const task = /^\[([ xX])\]\s+(.*)$/.exec(items[i].text);
      const body = task
        ? `<input type="checkbox" disabled ${task[1].trim() ? "checked" : ""} aria-label="${task[1].trim() ? "已完成" : "未完成"}"> ${inline(task[2], sourcePath)}`
        : inline(items[i].text, sourcePath);
      out.push(`<li>${body}</li>`);
      i += 1;
    }
    const tag = ordered ? "ol" : "ul";
    return { html: `<${tag}>${out.join("")}</${tag}>`, next: i };
  };
  const parts = [];
  let i = 0;
  while (i < items.length) {
    const result = build(i, items[i].indent);
    parts.push(result.html);
    i = result.next;
  }
  return parts.join("");
}

function isTableStartLine(lines, index) {
  return index + 1 < lines.length &&
    /^\s*\|.*\|\s*$/.test(lines[index]) &&
    /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/.test(lines[index + 1]);
}

function splitTableRow(line) {
  const text = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  const cells = [];
  let current = "";
  let code = false;
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] === "\\" && text[i + 1] === "|") { current += "|"; i += 1; continue; }
    if (text[i] === "`") code = !code;
    if (text[i] === "|" && !code) { cells.push(current.trim()); current = ""; continue; }
    current += text[i];
  }
  cells.push(current.trim());
  return cells;
}

function renderTable(block, sourcePath) {
  const rows = block.filter((_, i) => i !== 1).map(splitTableRow);
  if (!rows.length) return "";
  const [head, ...body] = rows;
  return `<div class="table-wrap"><table>
    <thead><tr>${head.map(cell => `<th scope="col">${inline(cell, sourcePath)}</th>`).join("")}</tr></thead>
    <tbody>${body.map(row => `<tr>${row.map(cell => `<td>${inline(cell, sourcePath)}</td>`).join("")}</tr>`).join("")}</tbody>
  </table></div>`;
}

function renderFigure(alt, href, sourcePath) {
  const url = safeUrl(resolveHref(href, sourcePath));
  const label = escapeHtml(alt || "文档图表");
  if (!url) return `<p class="muted">[图片已忽略：${label}]</p>`;
  return `<figure class="doc-figure">
    <a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer"><img src="${escapeHtml(url)}" alt="${label}" loading="lazy" decoding="async"></a>
    <figcaption>${label}</figcaption>
  </figure>`;
}

// Inline spans: code first (protected), then links, emphasis and <br>.
function inline(value, sourcePath) {
  const codes = [];
  let text = String(value ?? "").replace(/`([^`]+)`/g, (_, code) => {
    codes.push(`<code>${escapeHtml(code)}</code>`);
    return `\u0000${codes.length - 1}\u0000`;
  });
  text = escapeHtml(text);
  text = text.replace(/&lt;br\s*\/?&gt;/gi, "<br>");
  text = text.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_, alt, href) => {
    const url = safeUrl(resolveHref(unescapeHtml(href), sourcePath));
    return url ? `<img class="inline-img" src="${escapeHtml(url)}" alt="${alt}">` : alt;
  });
  text = text.replace(/\[([^\]]+)\]\(([^)\s]+)(?:\s+&quot;[^&]*&quot;)?\)/g, (_, label, href) => linkHtml(label, unescapeHtml(href), sourcePath));
  text = text.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  text = text.replace(/(^|[^*\w])\*([^*\s][^*]*)\*(?!\w)/g, "$1<em>$2</em>");
  text = text.replace(/~~([^~]+)~~/g, "<del>$1</del>");
  return text.replace(/\u0000(\d+)\u0000/g, (_, i) => codes[Number(i)]);
}

function linkHtml(label, href, sourcePath) {
  const [target, anchor = ""] = href.split("#");
  if (!/^[a-z][a-z\d+.-]*:/i.test(href) && /\.md$/i.test(target || "")) {
    const docPath = joinPath(sourcePath, decodeURIComponentSafe(target));
    const docId = docPath.replace(/\.md$/i, "");
    if (docIndex.has(docId)) {
      return `<a href="#view=reader&doc=${encodeURIComponent(docId)}" data-open-doc="${escapeHtml(docId)}"${anchor ? ` data-anchor="${escapeHtml(anchor)}"` : ""}>${label}</a>`;
    }
    return `<span class="broken-link" title="链接目标不存在：${escapeHtml(docPath)}">${label}</span>`;
  }
  if (!target && anchor) {
    return `<a href="#${escapeHtml(anchor)}" data-open-doc="${escapeHtml(state.activeDocId || "")}" data-anchor="${escapeHtml(anchor)}">${label}</a>`;
  }
  const url = safeUrl(resolveHref(href, sourcePath));
  if (!url) return label;
  const external = /^https?:/i.test(url);
  return `<a href="${escapeHtml(url)}"${external ? ` target="_blank" rel="noopener noreferrer"` : ""}>${label}</a>`;
}

// Paths in the data are relative to docs/; the HTML lives in docs/<outputDir>/.
function resolveHref(href, sourcePath) {
  if (!href || /^(?:[a-z][a-z\d+.-]*:|#|\/)/i.test(href)) return href;
  const joined = joinPath(sourcePath, href);
  return `${DATA.docsBase || ""}${joined.split("/").map(encodeURIComponent).join("/")}`;
}

function joinPath(sourcePath, href) {
  const base = String(sourcePath || "").split("/").slice(0, -1);
  for (const part of String(href).split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") base.pop();
    else base.push(part);
  }
  return base.join("/");
}

function safeUrl(url) {
  const value = String(url || "").trim();
  if (!value) return "";
  if (/^(https?:|mailto:|#|\.{0,2}\/|[^:]*$)/i.test(value)) return value;
  return "";
}

function unescapeHtml(value) {
  return String(value).replace(/&amp;/g, "&").replace(/&quot;/g, "\"").replace(/&#039;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">");
}

function decodeURIComponentSafe(value) {
  try { return decodeURIComponent(value); } catch { return value; }
}
