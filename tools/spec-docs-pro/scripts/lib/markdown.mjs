// Minimal, dependency-free Markdown helpers shared by generate-browser and check-docs.

const ROW = /^\s*\|.*\|\s*$/;
const SEPARATOR = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/;
const HEADING = /^(#{1,6})\s+(.+?)\s*#*\s*$/;

export function parseFrontmatter(source) {
  const text = String(source).replace(/^\uFEFF/, "");
  const match = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(\r?\n|$)/.exec(text);
  if (!match) return { data: {}, body: text, hasFrontmatter: false, lineCount: 0 };
  const data = {};
  for (const line of match[1].split(/\r?\n/)) {
    const kv = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(line);
    if (!kv) continue;
    let value = kv[2].trim();
    if (/^(["']).*\1$/.test(value)) value = value.slice(1, -1);
    data[kv[1]] = value;
  }
  return {
    data,
    body: text.slice(match[0].length),
    hasFrontmatter: true,
    lineCount: match[0].split(/\r?\n/).length - 1
  };
}

// Returns every line with a flag telling whether it sits inside a fenced code block.
export function scanLines(source) {
  const lines = String(source).replace(/\r\n/g, "\n").split("\n");
  let fence = null;
  return lines.map((line, index) => {
    const marker = /^\s*(`{3,}|~{3,})/.exec(line);
    if (marker) {
      const char = marker[1][0];
      if (!fence) {
        fence = { char, size: marker[1].length };
        return { line, index, inCode: true, fence: true };
      }
      if (char === fence.char && marker[1].length >= fence.size) {
        fence = null;
        return { line, index, inCode: true, fence: true };
      }
    }
    return { line, index, inCode: fence !== null, fence: false };
  });
}

export function extractHeadings(source) {
  const headings = [];
  for (const item of scanLines(source)) {
    if (item.inCode) continue;
    const match = HEADING.exec(item.line);
    if (match) headings.push({ level: match[1].length, text: cleanInline(match[2]), raw: match[2].trim(), line: item.index });
  }
  return headings;
}

export function extractTitle(source, fallback = "") {
  const fm = parseFrontmatter(source);
  if (fm.data.title) return cleanInline(fm.data.title);
  const h1 = extractHeadings(fm.body).find(heading => heading.level === 1);
  return h1 ? h1.text : fallback;
}

export function splitRow(line) {
  const text = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  const cells = [];
  let current = "";
  let inCode = false;
  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    if (char === "\\" && text[i + 1] === "|") {
      current += "|";
      i += 1;
      continue;
    }
    if (char === "`") inCode = !inCode;
    if (char === "|" && !inCode) {
      cells.push(current.trim());
      current = "";
      continue;
    }
    current += char;
  }
  cells.push(current.trim());
  return cells;
}

export function isTableStart(lines, index) {
  return index + 1 < lines.length && ROW.test(lines[index]) && SEPARATOR.test(lines[index + 1]);
}

// Every table with its nearest heading and its enclosing level-2 heading.
export function extractTables(source) {
  const items = scanLines(source);
  const tables = [];
  let heading = null;
  let h2 = null;
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    if (item.inCode) continue;
    const match = HEADING.exec(item.line);
    if (match) {
      heading = { level: match[1].length, text: cleanInline(match[2]), line: i };
      if (heading.level <= 2) h2 = heading;
      continue;
    }
    const next = items[i + 1];
    if (ROW.test(item.line) && next && !next.inCode && SEPARATOR.test(next.line)) {
      const header = splitRow(item.line).map(cleanInline);
      const rows = [];
      let j = i + 2;
      while (j < items.length && !items[j].inCode && ROW.test(items[j].line)) {
        rows.push(splitRow(items[j].line));
        j += 1;
      }
      tables.push({ header, rows, line: i, heading, h2 });
      i = j - 1;
    }
  }
  return tables;
}

// Sections keyed by heading; `lines` excludes the heading itself.
export function extractSections(source, level = 2) {
  const items = scanLines(source);
  const sections = [];
  let current = null;
  for (const item of items) {
    const match = item.inCode ? null : HEADING.exec(item.line);
    if (match && match[1].length <= level) {
      if (current) sections.push(current);
      current = match[1].length === level ? { title: cleanInline(match[2]), line: item.index, lines: [] } : null;
      continue;
    }
    if (current) current.lines.push(item);
  }
  if (current) sections.push(current);
  return sections;
}

export function extractLinks(source) {
  const links = [];
  const pattern = /(!?)\[([^\]]*)\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g;
  for (const item of scanLines(source)) {
    if (item.inCode) continue;
    const line = item.line.replace(/`[^`]*`/g, match => " ".repeat(match.length));
    for (const match of line.matchAll(pattern)) {
      links.push({ image: match[1] === "!", text: match[2], href: match[3], line: item.index });
    }
  }
  return links;
}

// First prose paragraph after the H1, used as a short "goal" description.
export function firstParagraph(source, maxLength = 90) {
  const body = parseFrontmatter(source).body;
  for (const item of scanLines(body)) {
    if (item.inCode) continue;
    const line = item.line.trim();
    if (!line || HEADING.test(line) || ROW.test(line) || /^[-*+>]|^\d+\.|^!\[|^---$/.test(line)) continue;
    return truncate(cleanInline(line), maxLength);
  }
  return "";
}

export function cleanInline(text) {
  return String(text || "")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/<br\s*\/?>/gi, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function truncate(text, maxLength) {
  const value = String(text || "");
  return value.length > maxLength ? `${value.slice(0, maxLength - 1)}…` : value;
}

export function normalizeName(text) {
  return cleanInline(text).replace(/演进$/, "").replace(/[\s、，,。.：:（）()·\-—_/]/g, "").toLowerCase();
}

// Dice coefficient over character bigrams; robust enough for short Chinese labels.
export function similarity(a, b) {
  const left = normalizeName(a);
  const right = normalizeName(b);
  if (!left || !right) return 0;
  if (left === right) return 1;
  if (left.startsWith(right) || right.startsWith(left)) return 0.9;
  const grams = value => {
    const result = new Map();
    for (let i = 0; i < value.length - 1; i += 1) {
      const gram = value.slice(i, i + 2);
      result.set(gram, (result.get(gram) || 0) + 1);
    }
    return result;
  };
  const x = grams(left);
  const y = grams(right);
  let overlap = 0;
  for (const [gram, count] of x) overlap += Math.min(count, y.get(gram) || 0);
  const total = [...x.values()].reduce((s, v) => s + v, 0) + [...y.values()].reduce((s, v) => s + v, 0);
  return total ? (2 * overlap) / total : 0;
}
