# 10. App domains (Phase 4)

Spec: 7.7, 6.4 domains. Builds on prompt 05.

Do:
- Domains per service: add, DNS check, certificate, primary, www redirect, force HTTPS, HSTS, remove. Multiple domains per app.
- Wire the "Domains & SSL" tab (`getDomains`, `addDomain`, `checkDns`, `removeDomain`, `makePrimary`).
- Renewal failures raise an attention item (alerts come in prompt 15).
- Internal URL `<service>` on the project network shown for reference.

Verify: point a test domain at the VM (or use a public test VM), add it, get a certificate, add `www` with redirect.
