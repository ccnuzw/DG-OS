export type RichRenderFailure = 'limit' | 'unsupported' | 'failed' | 'timeout' | 'unavailable';
export const MAX_DIAGRAM_CHARS = 8000;
export const MAX_MATH_CHARS = 4000;
export const MAX_RICH_OUTPUT_CHARS = 250000;
export const RICH_RENDER_TIMEOUT_MS = 5000;

/** Limits are checked before loading or invoking either rendering library. Not the output security boundary. */
export function diagramInputFailure(source: string): RichRenderFailure | null {
  if (source.length > MAX_DIAGRAM_CHARS || source.split('\n').length > 120) return 'limit';
  const lines = source.trim().split('\n');
  while (lines[0]?.trim().startsWith('%%') && !lines[0].trim().startsWith('%%{')) lines.shift();
  const header = lines.shift()?.trim() ?? '';
  const flowHeader = /^(?:flowchart|graph)\s+(?:TB|TD|BT|RL|LR)(?:\s*;\s*(.*))?\s*$/.exec(header);
  const flow = flowHeader !== null;
  const sequence = header === 'sequenceDiagram';
  if (flowHeader?.[1]) lines.unshift(flowHeader[1]);
  if (!flow && !sequence) return 'unsupported';
  // These features are deliberately outside the supported subset. CSP, strict mode and SVG reconstruction
  // independently prevent network/HTML/script execution even if a future Mermaid syntax bypasses this filter.
  if (/%%\{|[<][A-Za-z!/]|https?:|javascript:|data:|file:|ftp:|www\.|\\|&(?:#|[A-Za-z])|@\{|\$\$/im.test(source)) return 'unsupported';
  // Keywords inside node labels or sequence message text are ordinary prose, not commands.
  if (/(?:^|[;\n])[ \t]*(?:click|href|callback|style|classDef|linkStyle|image|icon|init|config)\b/i.test(source)) return 'unsupported';
  let relations = 0;
  const identities = new Set<string>();
  for (const raw of lines) {
    const line = raw.trim();
    if (!line || line.startsWith('%%')) continue;
    if (line.length > 500) return 'limit';
    if (flow) {
      // Bound implicit grouped-edge products before parsing; the native Mermaid database enforces the
      // exact node/edge limits again before layout. Source syntax is still interpreted by Mermaid itself.
      if (/\b(?:subgraph|direction|class)\b|[{}]{2}|[-=]{5}|[<>]{3}/i.test(line)) return 'unsupported';
      const groups = line.split(/-->|---|==>|-\.->|~~~/);
      relations += groups.slice(1).reduce((sum, group, index) => sum + (groups[index].split('&').length * group.split('&').length), 0);
    } else {
      const message = /^([A-Za-z_][\w-]*)\s*([<]?[\-]+[)>x]+|[<]+[\-]+)\s*([A-Za-z_][\w-]*)\s*:/.exec(line);
      const participant = /^(?:participant|actor)\s+([A-Za-z_][\w-]*)(?:\s+as\s+.+)?$/.exec(line);
      if (message) { identities.add(message[1]); identities.add(message[3]); relations++; }
      else if (participant) identities.add(participant[1]);
      else if (!/^(?:autonumber|activate\s+[\w-]+|deactivate\s+[\w-]+|Note\s+(?:right of|left of|over)\s+[\w, -]+:.+)$/i.test(line)) return 'unsupported';
    }
    if (identities.size > 60 || relations > 100) return 'limit';
  }
  return null;
}

/** Command scanning is supplemental: KaTeX trust=false, strict errors and MathML allowlists are authoritative. */
export function mathInputFailure(source: string): RichRenderFailure | null {
  if (source.length > MAX_MATH_CHARS) return 'limit';
  if (/\\(?:href|url|includegraphics|html\w*|def|gdef|edef|xdef|newcommand|renewcommand|let|futurelet|csname)\b/i.test(source)) return 'unsupported';
  let depth = 0;
  for (const character of source) {
    if (character === '{' && ++depth > 40) return 'limit';
    if (character === '}') depth--;
  }
  return null;
}

export interface SafeRichNode { tag: string; attributes: Record<string, string>; children: Array<SafeRichNode | string> }
const SVG_TAGS: Readonly<Record<string, true>> = { svg: true, g: true, defs: true, marker: true, path: true, rect: true, circle: true, ellipse: true, line: true, polyline: true, polygon: true, text: true, tspan: true, title: true, desc: true };
const SVG_ATTRIBUTES: Readonly<Record<string, true>> = { viewBox: true, x: true, y: true, x1: true, x2: true, y1: true, y2: true, width: true, height: true, rx: true, ry: true, r: true, cx: true, cy: true, d: true, points: true, transform: true, 'text-anchor': true, 'dominant-baseline': true, 'alignment-baseline': true, dx: true, dy: true, 'font-size': true, 'font-weight': true, markerWidth: true, markerHeight: true, refX: true, refY: true, orient: true, markerUnits: true, preserveAspectRatio: true, 'stroke-width': true, 'stroke-dasharray': true };
const MATH_TAGS: Readonly<Record<string, true>> = { math: true, semantics: true, annotation: true, mrow: true, mi: true, mo: true, mn: true, mtext: true, mspace: true, mfrac: true, msqrt: true, mroot: true, msup: true, msub: true, msubsup: true, munder: true, mover: true, munderover: true, mtable: true, mtr: true, mtd: true, menclose: true, mpadded: true, mphantom: true, mstyle: true, mmultiscripts: true, mprescripts: true, none: true };
const MATH_ATTRIBUTES: Readonly<Record<string, true>> = { display: true, mathvariant: true, stretchy: true, symmetric: true, fence: true, separator: true, accent: true, accentunder: true, columnalign: true, rowalign: true, columnspacing: true, rowspacing: true, columnlines: true, rowlines: true, linethickness: true, notation: true, encoding: true, scriptlevel: true, displaystyle: true, lspace: true, rspace: true, minsize: true, maxsize: true, width: true, height: true, depth: true, voffset: true, columnspan: true, rowspan: true };

/** Parse inert XML, then rebuild only known SVG/MathML primitives. Never insert library HTML into the app DOM. */
export function parseRichOutput(markup: string, kind: 'diagram' | 'math', prefix: string): SafeRichNode {
  if (markup.length > MAX_RICH_OUTPUT_CHARS) throw new Error('limit');
  if (/<!DOCTYPE|<!ENTITY/i.test(markup)) throw new Error('unsupported');
  const xml = new DOMParser().parseFromString(markup, 'application/xml');
  if (xml.querySelector('parsererror')) throw new Error('failed');
  const root = kind === 'diagram' ? xml.documentElement : xml.querySelector('math');
  const namespace = kind === 'diagram' ? 'http://www.w3.org/2000/svg' : 'http://www.w3.org/1998/Math/MathML';
  const tags = kind === 'diagram' ? SVG_TAGS : MATH_TAGS;
  const attributes = kind === 'diagram' ? SVG_ATTRIBUTES : MATH_ATTRIBUTES;
  if (!root || root.localName !== (kind === 'diagram' ? 'svg' : 'math') || root.namespaceURI !== namespace) throw new Error('failed');
  const markerIds = new Set<string>();
  if (kind === 'diagram') for (const marker of root.querySelectorAll('marker')) if (/^[\w-]{1,160}$/.test(marker.id)) markerIds.add(marker.id);
  let count = 0;
  function visit(element: Element, depth: number): SafeRichNode | null {
    if (++count > 4000 || depth > 60) throw new Error('limit');
    if (!Object.prototype.hasOwnProperty.call(tags, element.localName) || element.namespaceURI !== namespace) {
      // Mermaid 12 emits unused icon symbols and decorative shadow filters even for basic diagrams.
      // Drop those definition subtrees instead of admitting filter effects, use-links or HTML.
      if (kind === 'diagram' && (element.localName === 'style' || (element.parentElement?.localName === 'defs' && ['filter', 'symbol'].includes(element.localName)))) return null;
      throw new Error('unsupported');
    }
    const props: Record<string, string> = {};
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name;
      const value = attribute.value;
      if (name === 'xmlns' || name === 'xmlns:xlink') continue;
      if (/^on/i.test(name) || ['href', 'xlink:href', 'src'].includes(name)) throw new Error('unsupported');
      if (kind === 'diagram' && name === 'id' && markerIds.has(value)) props.id = `${prefix}-${value}`;
      else if (kind === 'diagram' && name === 'class') props.class = value.split(/\s+/).filter(value => /^[A-Za-z_][\w-]{0,80}$/.test(value)).join(' ');
      else if (kind === 'diagram' && name === 'style') {
        // Preserve geometry-related text properties only; never retain a CSS declaration or URL.
        for (const declaration of value.split(';')) {
          const [property, setting] = declaration.split(':').map(part => part.trim());
          if (property === 'text-anchor' && /^(?:start|middle|end)$/.test(setting ?? '')) props[property] = setting;
          else if (property === 'dominant-baseline' && /^(?:auto|middle|central|hanging|text-before-edge)$/.test(setting ?? '')) props[property] = setting;
          else if (property === 'font-weight' && /^(?:normal|bold|[1-9]00)$/.test(setting ?? '')) props[property] = setting;
        }
      }
      else if (kind === 'diagram' && name.startsWith('marker-')) {
        const id = /^url\(#([\w-]+)\)$/.exec(value)?.[1];
        if (id && markerIds.has(id) && ['marker-start', 'marker-mid', 'marker-end'].includes(name)) props[name] = `url(#${prefix}-${id})`;
        else throw new Error('unsupported');
      } else if (kind === 'diagram' && (name === 'fill' || name === 'stroke')) {
        if (value === 'none') props[name] = value;
      } else if (Object.prototype.hasOwnProperty.call(attributes, name) && !attribute.namespaceURI) {
        if (value.length > 20000 || /url|[<>]|[\\]/i.test(value)) throw new Error('unsupported');
        props[name] = value;
      }
      // Arbitrary CSS, data attributes and all remaining presentation metadata are discarded.
    }
    if (element === root && kind === 'diagram') {
      const dimensions = (props.viewBox ?? '').trim().split(/[\s,]+/).map(Number);
      if (dimensions.length !== 4 || dimensions.some(value => !Number.isFinite(value)) || dimensions[2] <= 0 || dimensions[3] <= 0 || dimensions[2] > 30000 || dimensions[3] > 30000) throw new Error('limit');
      delete props.width; delete props.height;
    }
    const children: SafeRichNode['children'] = [];
    for (const child of Array.from(element.childNodes)) {
      if (child.nodeType === 3) children.push(child.textContent ?? '');
      else if (child.nodeType === 1) { const node = visit(child as Element, depth + 1); if (node) children.push(node); }
    }
    return { tag: element.localName, attributes: props, children };
  }
  return visit(root, 0)!;
}

export function spreadsheetCell(value: string): string {
  return /^[\s\u0000-\u001f\u007f-\u009f\ufeff]*[=+\-@]/.test(value) || /^[\t\r\n]/.test(value) ? `'${value}` : value;
}
export function tableData(rows: readonly (readonly string[])[], delimiter: ',' | '\t'): string {
  return rows.map(row => row.map(cell => `"${spreadsheetCell(cell).replace(/"/g, '""')}"`).join(delimiter)).join('\r\n');
}
