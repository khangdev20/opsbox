// Plain-language text for the "opsbox updates" status in Settings. Pure functions (no DOM)
// so tools/check-update-status.mjs can run them under node.

/** "HH:MM" in the browser's time zone for a Unix-seconds time, or '' when there is none. */
export function clock(sec) {
  if (!Number.isFinite(sec) || sec <= 0) return '';
  const d = new Date(sec * 1000);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/**
 * What to say when `u` (the /updates/self snapshot) offers nothing.
 * Returns { text, detail } where `detail` is an optional second, quieter line.
 */
export function noOfferText(u) {
  const channel = u.checkedChannel || u.channel;
  switch (u.reason) {
    case 'dev_build':
      return { text: `opsbox ${u.current} · development build, not updated automatically` };
    case 'no_releases_on_channel':
      return {
        text: channel === 'stable'
          ? 'No stable release yet; switch to beta to get alpha releases'
          : `No release has been published on the ${channel} channel yet`,
      };
    case 'network_error':
      return {
        text: "Couldn't reach GitHub to look for updates. Check this server's internet connection, then press Check now.",
        detail: u.reasonDetail || '',
      };
    case 'rate_limited': {
      const at = clock(u.retryAt);
      return { text: `GitHub is limiting update checks right now. opsbox will try again${at ? ` after ${at}` : ' later'}.` };
    }
    case 'up_to_date':
      return { text: `opsbox ${u.current} · up to date${u.latest ? ` (newest on the ${channel} channel: ${u.latest.version})` : ''}` };
    default:
      // No check has run yet, or the server predates reasons.
      return u.checkedAt
        ? { text: `opsbox ${u.current} · up to date` }
        : { text: `opsbox ${u.current} · not checked yet. Press Check now.` };
  }
}

/** "last checked 14:05 · beta channel"; the time is left out before the first check. */
export function checkedLine(u) {
  const at = clock(u.checkedAt);
  return `${at ? `last checked ${at} · ` : ''}${u.channel} channel`;
}

/** Short message for the toast after "Check now". */
export function checkToast(u) {
  if (u.updateAvailable && u.latest) return `opsbox ${u.latest.version} is available.`;
  const t = noOfferText(u).text;
  return t.replace(/^opsbox \S+ · /, '').replace(/^\w/, (c) => c.toUpperCase());
}
