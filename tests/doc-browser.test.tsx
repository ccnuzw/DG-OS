// @vitest-environment node
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const dir = new URL('../docs/90-参考资料/', import.meta.url);
const decisionLog = new URL('../docs/06-决策记录/V1-冻结决策.md', import.meta.url);
const browserUrl = new URL('产品文档浏览器.html', dir);
const dataUrl = new URL('产品文档浏览器-文档数据.js', dir);
const VIEWS = ['overview', 'reader', 'features', 'map', 'evolution', 'timeline'];

type Feature = {
  id: string;
  title: string;
  acCount: number;
  track: string | null;
  implementation: { statusKey: string; statusLabel: string };
};
type Data = {
  docs: { id: string; path: string }[];
  features: Feature[];
  decisions: { id: string }[];
  versions: { id: string }[];
  warnings: unknown[];
  content: Record<string, string>;
};

let dom: JSDOM;
let data: Data;

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const content = () => dom.window.document.querySelector('#content')!;
async function openView(view: string) {
  (dom.window.document.querySelector(`[data-view="${view}"]`) as HTMLElement).click();
  await wait(0);
  return content();
}

beforeAll(async () => {
  const html = readFileSync(browserUrl, 'utf8');
  dom = new JSDOM(html, { url: browserUrl.href, runScripts: 'outside-only' });
  // File URLs have an opaque origin, so localStorage is unavailable; the viewer only needs these methods.
  Object.defineProperty(dom.window, 'localStorage', {
    value: { getItem: () => null, setItem: () => {} },
  });
  dom.window.eval(readFileSync(dataUrl, 'utf8'));
  for (const match of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
    if (match[1].trim()) dom.window.eval(match[1]);
  }
  dom.window.document.dispatchEvent(new dom.window.Event('DOMContentLoaded'));
  await wait(50);
  data = (dom.window as unknown as { SPEC_DOCS_DATA: Data }).SPEC_DOCS_DATA;
});

afterAll(() => dom?.window.close());

describe('generated data', () => {
  it('covers every Markdown document and all ten V1 features', () => {
    expect(data.docs.length).toBeGreaterThanOrEqual(66);
    for (const doc of data.docs) expect(data.content[doc.id], doc.path).toBeTruthy();
    expect(data.features.map((feature) => feature.id)).toEqual(
      Array.from({ length: 10 }, (_, i) => `V1-FR-${String(i + 1).padStart(3, '0')}`),
    );
  });

  it('has no data-quality warnings and links every feature to its status and track', () => {
    expect(data.warnings).toEqual([]);
    for (const feature of data.features) {
      expect(feature.implementation.statusKey, feature.id).not.toBe('unknown');
      expect(feature.track, feature.id).toBeTruthy();
      expect(feature.acCount, feature.id).toBeGreaterThan(0);
    }
  });

  it('never carries placeholder text in feature, decision or version fields', () => {
    const visible = JSON.stringify({
      features: data.features,
      decisions: data.decisions,
      versions: data.versions,
    });
    expect(visible).not.toMatch(/"(?:undefined|NaN|null)"|undefined|\[object Object\]/);
  });

  it('lists V1 through V8 and every frozen decision', () => {
    expect(data.versions.map((version) => version.id)).toEqual(
      Array.from({ length: 8 }, (_, i) => `V${i + 1}`),
    );
    const log = readFileSync(decisionLog, 'utf8');
    const ids = [...log.matchAll(/^## (D\d{3}) /gm)].map((match) => match[1]);
    expect(data.decisions.map((decision) => decision.id)).toEqual(ids);
  });
});

describe('viewer', () => {
  it('finishes loading and reports the document and feature counts', () => {
    expect(dom.window.document.querySelector('#loadStatus')?.textContent).toMatch(/\d+ 份文档 · 10 个功能/);
  });

  it('renders every view without placeholder text', async () => {
    for (const view of VIEWS) {
      const text = (await openView(view)).textContent ?? '';
      expect(text.length, view).toBeGreaterThan(200);
      expect(text, view).not.toMatch(/undefined|NaN|\[object Object\]/);
    }
  });

  it('draws the version roadmap and the capability heat map from the data', async () => {
    const map = await openView('map');
    for (let i = 1; i <= 8; i += 1) expect(map.textContent).toContain(`V${i}`);
    expect(map.querySelectorAll('table, svg, .heat-cell, .heatmap').length).toBeGreaterThan(0);
  });

  it('shows all ten features in the feature view', async () => {
    const text = (await openView('features')).textContent ?? '';
    for (const feature of data.features) expect(text, feature.id).toContain(feature.id);
  });

  it('opens a document by deep link and renders its headings', async () => {
    (dom.window.document.querySelector('#docList a, #docList button') as HTMLElement).click();
    await wait(0);
    expect(content().querySelector('h1, h2')).not.toBeNull();
  });
});

describe('table styling', () => {
  // jsdom has no layout engine, but it parses the embedded stylesheet. A header cell that no padding rule
  // matches is rendered unstyled, which is how the feature list and evolution matrix once looked.
  function paddingRuleFor(element: Element) {
    for (const sheet of Array.from(dom.window.document.styleSheets)) {
      for (const rule of Array.from(sheet.cssRules) as CSSStyleRule[]) {
        if (!rule.selectorText || !rule.style) continue;
        try {
          if (element.matches(rule.selectorText) && rule.style.padding) return rule;
        } catch {
          // selector jsdom cannot evaluate
        }
      }
    }
    return null;
  }

  it.each(['overview', 'features', 'evolution'])('styles the table header cells of the %s view', async (view) => {
    const th = (await openView(view)).querySelector('table th');
    expect(th, `${view} renders a table`).not.toBeNull();
    expect(paddingRuleFor(th!), `${view} table header has a padding rule`).not.toBeNull();
  });
});
