// Runs synchronously in <head>, before any framework code. Keep it plain ES5: no modules, no build step.
// 1. One-use secrets (the Accounts callback ?state=&code=, the live invitation #grant=) leave the address bar and history
//    before Next.js or the app reads the URL. The app reads the captured entry URL exactly once (src/lib/entry.ts).
// 2. The theme is applied before first paint so the page never flashes the wrong colours.
(function () {
  var href = location.href;
  if (location.search || location.hash) history.replaceState(null, '', location.pathname);
  Object.defineProperty(window, '__sbEntry', { value: href, configurable: true });

  var theme = null;
  try { theme = localStorage.getItem('silicon-browser:theme'); } catch (error) { theme = null; }
  if (theme !== 'light' && theme !== 'dark') {
    theme = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  document.documentElement.setAttribute('data-theme', theme);
})();
