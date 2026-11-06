// These functions execute inside the real Chrome page, not jsdom or a source-wiring test.
export function installProbe() {
  const require = (condition, message) => { if (!condition) throw new Error(message); };
  const one = selector => {
    const element = document.querySelector(selector);
    require(element instanceof HTMLElement, `Missing production surface: ${selector}`);
    return element;
  };
  const box = element => {
    const { x, y, width, height, top, right, bottom, left } = element.getBoundingClientRect();
    return { x, y, width, height, top, right, bottom, left };
  };
  const visible = (element, label, ring = 0) => {
    const r = box(element);
    require(r.width > 0 && r.height > 0, `${label}: empty border box`);
    require(r.left >= ring - 1 && r.top >= ring - 1 && r.right <= innerWidth - ring + 1 && r.bottom <= innerHeight - ring + 1, `${label}: outside viewport ${JSON.stringify(r)}`);
    const style = getComputedStyle(element);
    require(style.visibility !== 'hidden' && style.display !== 'none', `${label}: hidden`);
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      const css = getComputedStyle(parent), p = box(parent);
      if (/(auto|scroll|hidden|clip)/.test(css.overflowX)) require(r.left >= p.left + parent.clientLeft + ring - 1 && r.right <= p.left + parent.clientLeft + parent.clientWidth - ring + 1, `${label}: horizontally clipped by ${parent.className}`);
      if (/(auto|scroll|hidden|clip)/.test(css.overflowY)) require(r.top >= p.top + parent.clientTop + ring - 1 && r.bottom <= p.top + parent.clientTop + parent.clientHeight - ring + 1, `${label}: vertically clipped by ${parent.className}`);
    }
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    require(hit && (hit === element || element.contains(hit) || hit.contains(element)), `${label}: obscured by another surface`);
    return r;
  };
  const textFits = (element, label) => {
    require(element.scrollWidth <= element.clientWidth + 1, `${label}: horizontal text overflow`);
    const range = document.createRange();
    range.selectNodeContents(element);
    const outer = box(element);
    for (const rect of range.getClientRects()) {
      require(rect.left >= outer.left - 1 && rect.right <= outer.right + 1 && rect.top >= outer.top - 1 && rect.bottom <= outer.bottom + 1, `${label}: text is clipped`);
    }
  };
  window.__visualProbe = { require, one, box, visible, textFits };
}

export function inspectControls(expected) {
  const { require, one, visible } = window.__visualProbe;
  const fonts = {};
  for (const domain of ['ui', 'reading', 'code']) {
    const wrapper = one(`[data-preview-input-domain="${domain}"]`);
    const field = wrapper.matches('input,textarea') ? wrapper : wrapper.querySelector('input,textarea');
    require(field, `Missing ${domain} input`);
    visible(field, `${domain} input`);
    const css = getComputedStyle(field);
    const expectedSize = expected[`${domain}Size`] + (domain === 'ui' ? 1 : 0); // Production text-sm is UI preference + 1px.
    require(Math.abs(parseFloat(css.fontSize) - expectedSize) < 0.1, `${domain} font size ${css.fontSize}, expected ${expectedSize}px`);
    fonts[domain] = { size: css.fontSize, family: css.fontFamily, lineHeight: css.lineHeight };
  }
  require(fonts.code.family !== fonts.ui.family, 'Code input must retain a distinct monospace domain');
  return { fonts };
}

export function inspectSelect() {
  const { require, one, visible, textFits } = window.__visualProbe;
  const menu = one('[role="listbox"]');
  const menuBox = visible(menu, 'Select menu');
  require(menu.scrollWidth <= menu.clientWidth + 1, 'Select menu has horizontal overflow');
  const options = [...menu.querySelectorAll('[role="option"]')];
  require(options.length >= 2, 'Select fixture must exercise multiple options');
  const longest = options.reduce((a, b) => a.textContent.length > b.textContent.length ? a : b);
  longest.scrollIntoView({ block: 'nearest' });
  visible(longest, 'Longest Select option');
  textFits(longest, 'Longest Select option');
  const r = longest.getBoundingClientRect();
  const range = document.createRange(); range.selectNodeContents(longest);
  for (const line of range.getClientRects()) require(line.right <= menuBox.right + 1 && line.left >= menuBox.left - 1 && line.top >= menu.getBoundingClientRect().top - 1 && line.bottom <= menu.getBoundingClientRect().bottom + 1, 'Long Select label extends outside menu');
  return { menu: menuBox, option: { width: r.width, height: r.height }, options: options.length, longest: longest.textContent };
}

export function inspectModal() {
  const { require, one, visible } = window.__visualProbe;
  const dialog = one('[role="dialog"]');
  const rect = visible(dialog, 'Modal', 8);
  require(dialog.scrollWidth <= dialog.clientWidth + 1, 'Modal horizontal overflow');
  const buttons = [...dialog.querySelectorAll('.ui-modal-footer button')];
  require(buttons.length >= 2, 'Expected real Modal confirm/cancel controls');
  buttons.forEach((button, index) => visible(button, `Modal action ${index}`));
  require(dialog.contains(document.activeElement), 'Modal keyboard focus escaped dialog');
  if (document.activeElement.matches('input,textarea,button')) visible(document.activeElement, 'Focused Modal control', 2);
  return { dialog: rect, active: document.activeElement.id || document.activeElement.textContent };
}

export function inspectSettings() {
  const { require, one, visible } = window.__visualProbe;
  const scroller = one('[data-settings-tab-scroller]');
  const active = document.activeElement;
  require(active?.matches('.workspace-settings-category[role="tab"]'), 'Arrow key must move focus to settings category tab');
  const ringStyle = getComputedStyle(active);
  const ring = Math.max(2, (parseFloat(ringStyle.outlineWidth) || 0) + (parseFloat(ringStyle.outlineOffset) || 0));
  const rect = visible(active, 'Settings category focus', ring);
  const parent = scroller.getBoundingClientRect();
  require(rect.top >= parent.top + scroller.clientTop + ring - 1 && rect.bottom <= parent.top + scroller.clientTop + scroller.clientHeight - ring + 1, 'Settings category focus ring clipped by scrollport');
  return { focus: active.textContent, rect, scrollTop: scroller.scrollTop };
}

export function inspectChat(approval) {
  const { require, one, visible } = window.__visualProbe;
  const composer = one('.chat-composer');
  const input = one('.chat-composer textarea');
  const rect = visible(input, 'Chat composer input');
  require(composer.scrollWidth <= composer.clientWidth + 1, 'Chat composer horizontal overflow');
  const submit = one('.chat-composer-primary button');
  visible(submit, 'Chat primary action');
  const result = { input: rect, action: submit.textContent };
  if (approval) {
    const buttons = [...document.querySelectorAll('.chat-approval-decisions button')];
    require(buttons.length >= 2, 'Expected real approval/reject buttons');
    result.approval = buttons.map((button, index) => ({ label: button.textContent, rect: visible(button, `Approval action ${index}`) }));
    require(document.querySelector('.chat-approval').contains(document.activeElement), 'Approval keyboard focus escaped card');
  } else {
    require(document.activeElement === input, 'Chat input did not receive keyboard focus');
    require(input.value.includes('\n'), 'Shift+Enter must insert a newline');
  }
  return result;
}
