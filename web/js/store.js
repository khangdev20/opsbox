// Tiny app state: theme + demo session. Persisted in localStorage/sessionStorage.
const THEME_KEY = 'opsbox.theme';
const safe = (fn, fallback) => { try { return fn(); } catch { return fallback; } };

export function initialTheme() {
  const saved = safe(() => localStorage.getItem(THEME_KEY), null);
  if (saved === 'light' || saved === 'dark') return saved;
  return window.matchMedia && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}
export function applyTheme(t) {
  document.documentElement.dataset.theme = t;
  safe(() => localStorage.setItem(THEME_KEY, t));
}
export const theme = () => document.documentElement.dataset.theme || 'dark';
export const toggleTheme = () => applyTheme(theme() === 'light' ? 'dark' : 'light');

// Not persisted: asked of the server on every page load (see main.js).
export const server = { setupRequired: false, role: '' };

export const session = {
  get user() { return safe(() => sessionStorage.getItem('opsbox.user'), null); },
  set(u) { safe(() => sessionStorage.setItem('opsbox.user', u)); },
  clear() { safe(() => sessionStorage.removeItem('opsbox.user')); },
};

// tick subscribers (called every 2 s by main.js)
export const listeners = new Set();
