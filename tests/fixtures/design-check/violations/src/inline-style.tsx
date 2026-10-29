export function Inline({ gap }: { gap: number }) {
  const el = document.createElement('div');
  el.style.color = 'inherit';
  el.setAttribute('style', 'margin: 0');
  const sheet = document.createElement('style');
  document.adoptedStyleSheets = [];
  CSS_SHEET.insertRule('a {}');
  const props = { style: { gap } };
  const h = (..._args: unknown[]) => null;
  h('div', { style: { gap } });
  return <div style={{ gap }} data-el={String(!!el && !!sheet && !!props)} />;
}
declare const CSS_SHEET: CSSStyleSheet;
