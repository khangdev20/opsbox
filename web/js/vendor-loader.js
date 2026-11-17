// Monaco and xterm both support AMD. Use one loader so a visit to Files cannot
// change how the Terminal page's UMD bundles export themselves.
let pending;
export function loadAmd() {
  if (window.define?.amd && window.require?.config) return Promise.resolve(window.require);
  if (!pending) pending = new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = new URL('../vendor/monaco/vs/loader.js', import.meta.url).href;
    script.onload = () => resolve(window.require);
    script.onerror = () => { pending = null; reject(new Error('Could not load the interface library.')); };
    document.head.append(script);
  });
  return pending;
}
