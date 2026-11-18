// Reveal / copy helpers for masked secrets (prompt 16o).
// Re-auth opens a 5-minute window; revealed values hide after 30 s or on navigation.
import { formDialog, toast } from './ui.js';
import * as api from './api.js';

const HIDE_MS = 30_000;
let hideTimer = 0;
let hideFn = null;

/** Clear any live revealed value (navigation or timeout). */
export function hideRevealed() {
  if (hideTimer) {
    clearTimeout(hideTimer);
    hideTimer = 0;
  }
  if (hideFn) {
    const fn = hideFn;
    hideFn = null;
    try { fn(); } catch (_) { /* ignore */ }
  }
}

if (typeof window !== 'undefined') {
  window.addEventListener('hashchange', hideRevealed);
}

/**
 * Ensure the 5-minute re-auth window is open. Shows a password / 2FA modal when needed.
 * @returns {Promise<boolean>} false if the user cancelled
 */
export async function ensureReauth({ title = 'Confirm it is you', body = 'Enter your account password or a two-factor code to show this secret.' } = {}) {
  const out = await formDialog({
    title,
    body,
    fields: [
      { name: 'password', label: 'Password', type: 'password', placeholder: 'Account password' },
      { name: 'code', label: 'Or two-factor code', placeholder: '123456' },
    ],
    submit: 'Continue',
    run: async (v) => {
      const password = (v.password || '').trim();
      const code = (v.code || '').trim();
      if (!password && !code) {
        const e = new Error('Enter your password or a two-factor code.');
        e.field = 'password';
        throw e;
      }
      await api.reauth({ password: password || undefined, code: code || undefined });
      return true;
    },
  });
  return !!out;
}

/**
 * Call `fetchValue` (which hits a reveal endpoint). On 403, prompt for re-auth and retry once.
 * `onValue(plaintext)` receives the secret; it is cleared after 30 s or navigation.
 * Prefer Copy (copyPlain) so the value need not stay in the DOM.
 */
export async function withReveal(fetchValue, onValue) {
  hideRevealed();
  let value;
  try {
    value = await fetchValue({});
  } catch (e) {
    if (!/confirm|password|two-factor|code|re-auth|reauth/i.test(String(e.message || e))) throw e;
    if (!await ensureReauth()) return null;
    value = await fetchValue({});
  }
  const plain = typeof value === 'string' ? value : value;
  onValue(plain);
  hideFn = () => onValue(null);
  hideTimer = setTimeout(hideRevealed, HIDE_MS);
  return plain;
}

/** Copy plaintext without inserting it into the page. */
export async function copyPlain(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast('Copied.');
  } catch (_) {
    // Fallback: short-lived off-DOM textarea.
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.cssText = 'position:fixed;left:-9999px;top:0';
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand('copy');
      toast('Copied.');
    } catch (e) {
      toast(e.message || 'Could not copy.', 'err');
    }
    ta.remove();
  }
}
