// @vitest-environment node
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

const browserUrl = new URL('../docs/90-参考资料/产品文档浏览器.html', import.meta.url);
let dom: JSDOM;

beforeAll(async () => {
  const html = readFileSync(browserUrl, 'utf8');
  dom = new JSDOM(html, { url: browserUrl.href, runScripts: 'outside-only' });
  // File URLs have an opaque origin; the standalone page only needs these storage methods.
  Object.defineProperty(dom.window, 'localStorage', {
    value: { getItem: () => null, setItem: () => {} },
  });
  dom.window.eval(readFileSync(new URL('产品文档浏览器-文档数据.js', browserUrl), 'utf8'));
  for (const match of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
    if (match[1].trim()) dom.window.eval(match[1]);
  }
  await vi.waitFor(() => {
    expect(dom.window.document.querySelector('#loadStatus')?.textContent).toContain('份 PRD');
  });
});

afterAll(() => dom?.window.close());

function openView(id: string) {
  dom.window.document.getElementById(id)!.click();
  return dom.window.document.querySelector('#content')!;
}

describe('DGOS document browser diagrams', () => {
  it('renders the project diagrams in both overview and map views', () => {
    const overview = openView('tabOverview');
    expect(overview.querySelectorAll('.diagram svg')).toHaveLength(2);
    const map = openView('tabMap');
    expect(map.querySelectorAll('.diagram svg')).toHaveLength(5);
    for (const svg of map.querySelectorAll('.diagram svg')) {
      expect(svg.getAttribute('aria-label')).toContain('DGOS');
    }
    expect(map.textContent).not.toMatch(/TUZAI|协议网关|充值订单|供应商账务/);
  });

  it('positions every roadmap connector and description in its own version row', () => {
    const map = openView('tabMap');
    const rows = [...map.querySelectorAll('.roadmap-row')];
    expect(rows.map(row => row.getAttribute('data-version'))).toEqual(
      Array.from({ length: 8 }, (_, i) => `V${i + 1}`),
    );
    let previousBottom = 0;
    for (const row of rows) {
      const rect = row.querySelector('rect')!;
      const top = Number(rect.getAttribute('y'));
      const bottom = top + Number(rect.getAttribute('height'));
      const center = (top + bottom) / 2;
      const line = row.querySelector('line')!;
      expect(top).toBeGreaterThan(previousBottom);
      expect(Number(line.getAttribute('y1'))).toBe(center);
      expect(Number(line.getAttribute('y2'))).toBe(center);
      expect(Number(line.getAttribute('x1'))).toBe(
        Number(rect.getAttribute('x')) + Number(rect.getAttribute('width')),
      );
      const description = row.querySelectorAll('text')[1];
      expect(Number(description.getAttribute('y'))).toBeGreaterThan(top);
      expect(Number(description.getAttribute('y')) + 20).toBeLessThan(bottom);
      expect(description.querySelectorAll('tspan')).toHaveLength(2);
      previousBottom = bottom;
    }
    const svg = rows[0].closest('svg')!;
    const spine = svg.querySelector(':scope > line')!;
    expect(Number(spine.getAttribute('x1'))).toBeLessThan(
      Number(rows[0].querySelector('rect')!.getAttribute('x')),
    );
    const footer = svg.querySelector(':scope > text:last-child')!;
    const height = Number(svg.getAttribute('viewBox')!.split(' ')[3]);
    expect(Number(footer.getAttribute('y'))).toBeGreaterThan(previousBottom);
    expect(Number(footer.getAttribute('y'))).toBeLessThan(height - 12);
  });

  it('uses explicit lines and sufficient vertical space for risk and domain cards', () => {
    const map = openView('tabMap');
    expect(map.querySelectorAll('.risk-card')).toHaveLength(6);
    expect(map.querySelectorAll('.data-domain-card')).toHaveLength(7);
    for (const card of map.querySelectorAll('.risk-card, .data-domain-card')) {
      const rect = card.querySelector('rect')!;
      const bottom = Number(rect.getAttribute('y')) + Number(rect.getAttribute('height'));
      const body = card.querySelectorAll('text')[1];
      const lines = [...body.querySelectorAll('tspan')];
      expect(lines.length).toBeGreaterThan(0);
      const lastBaseline = Number(body.getAttribute('y'))
        + lines.reduce((total, line) => total + Number(line.getAttribute('dy')), 0);
      expect(lastBaseline).toBeLessThan(bottom - 12);
    }
    for (const card of map.querySelectorAll('.risk-card')) {
      expect(card.querySelectorAll('text')[1].querySelectorAll('tspan').length).toBeGreaterThanOrEqual(2);
    }
  });

  it('escapes line content while retaining explicit SVG line coordinates', () => {
    const markup = dom.window.eval('diagramTextLines(["<unsafe>&", "second line"], 40, 60)');
    const host = dom.window.document.createElement('div');
    host.innerHTML = `<svg>${markup}</svg>`;
    expect(host.querySelector('unsafe')).toBeNull();
    expect(host.querySelector('tspan')?.textContent).toBe('<unsafe>&');
    expect([...host.querySelectorAll('tspan')].map(line => [line.getAttribute('x'), line.getAttribute('dy')]))
      .toEqual([['40', '0'], ['40', '18']]);
  });
});
