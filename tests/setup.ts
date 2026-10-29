// jsdom gaps that Radix primitives hit (D066 Phase 1 spike, found with tests/radix-jsdom.test.tsx).
// Each entry is installed only when jsdom lacks it, and does the least that lets Radix run. jsdom does
// no layout, so none of these make geometry-dependent behaviour (positioning, scrolling) testable:
// that is verified in a real WebView instead.
//
// Test files that opt into `@vitest-environment node` have no `window`; leave those untouched.
if (typeof window !== 'undefined') {
  // Select scrolls the highlighted option into view when it opens (keyboard path).
  Element.prototype.scrollIntoView ??= function scrollIntoView() {};

  // Select's pointer handling asks whether the pointer is captured (pointer path).
  Element.prototype.hasPointerCapture ??= function hasPointerCapture() {
    return false;
  };
  Element.prototype.setPointerCapture ??= function setPointerCapture() {};
  Element.prototype.releasePointerCapture ??= function releasePointerCapture() {};

  // Popper measures its arrow (and Select its content) through ResizeObserver; it never reports a size here.
  globalThis.ResizeObserver ??= class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
