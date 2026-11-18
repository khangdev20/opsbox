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
export const server = {
  setupRequired: false,
  role: '',
  displayName: null,
  totp: { enabled: true, method: 'app' },
};

export const session = {
  get user() { return safe(() => sessionStorage.getItem('opsbox.user'), null); },
  set(u) { safe(() => sessionStorage.setItem('opsbox.user', u)); },
  clear() { safe(() => sessionStorage.removeItem('opsbox.user')); },
};

/** Apply identity fields from a /session (or login) response onto `server` + `session`. */
export function applySession(s) {
  server.setupRequired = !!s.setupRequired;
  server.role = s.role || '';
  server.displayName = s.displayName || null;
  server.totp = s.totp && typeof s.totp === 'object'
    ? { enabled: !!s.totp.enabled, method: s.totp.method || null }
    : { enabled: true, method: 'app' };
  if (s.authenticated || (s.user && !('authenticated' in s))) session.set(s.user);
  else if (s.authenticated === false) session.clear();
}

// tick subscribers (called every 2 s by main.js)
export const listeners = new Set();
