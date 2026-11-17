// Renders an otpauth:// URI as a QR code in the browser. The library is vendored
// (web/vendor/qrcode, MIT) and loaded only when a page needs it; nothing is sent
// anywhere, so the TOTP secret never leaves this page.
let lib;
export async function qrSvg(text, { cell = 6, margin = 3 } = {}) {
  lib ||= (await import('../vendor/qrcode/qrcode.mjs')).qrcode;
  const q = lib(0, 'M');
  q.addData(text);
  q.make();
  const n = q.getModuleCount(), size = (n + margin * 2) * cell;
  let d = '';
  for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (q.isDark(r, c)) d += `M${(c + margin) * cell},${(r + margin) * cell}h${cell}v${cell}h-${cell}z`;
  // White background and black modules on purpose: scanners need the contrast, in both themes.
  return `<svg viewBox="0 0 ${size} ${size}" width="200" height="200" role="img" aria-label="QR code for your authenticator app" shape-rendering="crispEdges"><rect width="${size}" height="${size}" fill="#fff"/><path d="${d}" fill="#000"/></svg>`;
}

// "ABCD EFGH ..." so a secret is easy to read and type.
export const groupSecret = (s) => s.replace(/(.{4})/g, '$1 ').trim();
