// Screenshot surfaces are the production components rendered by frame.html.
// Keep the matrix small: each case covers a different user-visible failure mode.
export const cases = [
  { id: 'controls-long-select', scenario: 'controls', width: 1280, height: 900, locale: 'zh-CN', theme: 'light', density: 90, uiSize: 16, readingSize: 20, codeSize: 18 },
  { id: 'modal-narrow', scenario: 'modal', width: 520, height: 700, locale: 'en-US', theme: 'dark', density: 120, uiSize: 16, readingSize: 20, codeSize: 18 },
  { id: 'settings-category-focus', scenario: 'settings', width: 1280, height: 900, locale: 'en-US', theme: 'light', density: 120, uiSize: 16, readingSize: 20, codeSize: 18 },
  { id: 'chat-narrow-composer', scenario: 'chat', width: 520, height: 700, locale: 'zh-CN', theme: 'dark', density: 100, uiSize: 16, readingSize: 20, codeSize: 18 },
  { id: 'chat-narrow-approval', scenario: 'chat-approval', width: 520, height: 700, locale: 'en-US', theme: 'light', density: 90, uiSize: 16, readingSize: 20, codeSize: 18 },
];

// Pixelmatch's perceptual colour threshold tolerates tiny raster noise, not layout movement.
// A baseline image may differ by at most 0.01% of pixels; all geometry assertions are exact to 1 CSS px.
export const comparison = { threshold: 0.1, maxDifferentRatio: 0.0001, includeAA: true };
