/* Bootstrap for Monaco workers under CSP script-src 'self' (no data: URLs). */
self.MonacoEnvironment = {
  baseUrl: new URL("vs/", self.location.href).href,
};
importScripts(new URL("vs/base/worker/workerMain.js", self.location.href).href);
