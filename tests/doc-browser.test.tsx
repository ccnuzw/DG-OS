// @vitest-environment node
import { readdirSync, readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';

const dir = new URL('../docs/90-参考资料/', import.meta.url);
const specRoot = new URL('../docs/03-功能规格/', import.meta.url);
// Include every version's frozen-decision log, matching the generator's document inventory.
const decisionRoot = new URL('../docs/06-决策记录/', import.meta.url);
const decisionLogs = readdirSync(decisionRoot).filter(name => /^V\d+-冻结决策\.md$/.test(name)).sort().map(name => new URL(name, decisionRoot));
const browserUrl = new URL('产品文档浏览器.html', dir);
const dataUrl = new URL('产品文档浏览器-文档数据.js', dir);
const VIEWS = ['overview', 'reader', 'features', 'map', 'evolution', 'timeline'];

/** Feature IDs of the feature main documents on disk (technical designs share the ID and are not features). */
function featureIdsOnDisk(): string[] {
  const ids = readdirSync(specRoot, { recursive: true, encoding: 'utf8' })
    .filter((path) => path.endsWith('.md') && !path.endsWith('-技术设计.md'))
    .map((path) => readFileSync(new URL(path, specRoot), 'utf8').match(/^feature_id: *(V\d+-FR-\d{3})\s*$/m)?.[1])
    .filter((id): id is string => Boolean(id));
  return [...new Set(ids)].sort();
}

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
const runtimeErrors: unknown[] = [];

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const content = () => dom.window.document.querySelector('#content')!;
async function openView(view: string) {
  (dom.window.document.querySelector(`[data-view="${view}"]`) as HTMLElement).click();
  await wait(0);
  expect(new URLSearchParams(dom.window.location.hash.slice(1)).get('view')).toBe(view);
  return content();
}

beforeAll(async () => {
  const html = readFileSync(browserUrl, 'utf8');
  // jsdom 30 compares file-URL path arrays by identity in History-impl.js, rejecting even hash-only
  // pushState/replaceState. Use an HTTP origin with the real Unicode pathname; real file viewers can
  // rewrite fragments, so changing the viewer's offline history handling would hide an emulator bug.
  dom = new JSDOM(html, { url: `https://docs.dgos.test${browserUrl.pathname}`, runScripts: 'outside-only' });
  dom.window.addEventListener('error', (event) => runtimeErrors.push(event.error));
  // Scripts are evaluated manually below. Let jsdom's own lifecycle finish before registering init,
  // then dispatch DOMContentLoaded once so viewer event handlers are not installed twice.
  const loaded = Promise.withResolvers<void>();
  dom.window.addEventListener('load', () => loaded.resolve(), { once: true });
  await loaded.promise;
  dom.window.eval(readFileSync(dataUrl, 'utf8'));
  for (const match of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
    if (match[1].trim()) dom.window.eval(match[1]);
  }
  dom.window.document.dispatchEvent(new dom.window.Event('DOMContentLoaded'));
  await wait(50);
  data = (dom.window as unknown as { SPEC_DOCS_DATA: Data }).SPEC_DOCS_DATA;
});

afterEach(() => expect(runtimeErrors).toEqual([]));
afterAll(() => dom?.window.close());

describe('generated data', () => {
  it('covers every Markdown document and every feature main document', () => {
    expect(data.docs.length).toBeGreaterThanOrEqual(66);
    for (const doc of data.docs) expect(data.content[doc.id], doc.path).toBeTruthy();
    const onDisk = featureIdsOnDisk();
    expect(onDisk).toEqual(expect.arrayContaining(
      Array.from({ length: 10 }, (_, i) => `V1-FR-${String(i + 1).padStart(3, '0')}`),
    ));
    expect(data.features.map((feature) => feature.id)).toEqual(onDisk);
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
    const ids = decisionLogs.flatMap((log) =>
      [...readFileSync(log, 'utf8').matchAll(/^## (D\d{3}) /gm)].map((match) => match[1]),
    );
    expect(data.decisions.map((decision) => decision.id).sort()).toEqual(ids.sort());
  });
});

describe('viewer', () => {
  it('finishes loading and reports the document and feature counts', () => {
    expect(dom.window.document.querySelector('#loadStatus')?.textContent).toContain(
      `${data.docs.length} 份文档 · ${data.features.length} 个功能`,
    );
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

  it('shows every feature in the feature view', async () => {
    const text = (await openView('features')).textContent ?? '';
    for (const feature of data.features) expect(text, feature.id).toContain(feature.id);
  });

  it('opens a document by deep link and renders its headings', async () => {
    (dom.window.document.querySelector('#docList a, #docList button') as HTMLElement).click();
    await wait(0);
    expect(content().querySelector('h1, h2')).not.toBeNull();
    const hash = new URLSearchParams(dom.window.location.hash.slice(1));
    expect(hash.get('view')).toBe('reader');
    expect(data.docs.some((doc) => doc.id === hash.get('doc'))).toBe(true);
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
