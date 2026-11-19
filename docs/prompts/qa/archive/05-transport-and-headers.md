# QA 05. TLS, cookies and security headers

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Run `scripts/check-transport.sh` against the VM.
2. `curl -skI https://<vm>:8443/` and a few API paths: `Strict-Transport-Security`, `Content-Security-Policy`, `X-Content-Type-Options`, `Referrer-Policy`, `Cache-Control: no-store` on authenticated JSON.
3. Login response cookie: `__Host-` name, `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, no `Domain`.
4. Plain HTTP to the panel port on a non-loopback address is refused or redirected, never served.
5. A forged `X-Forwarded-For` from outside the proxy is ignored (audit log shows the real IP).

## Pass when
Every line above is true; fix a missing header only if it is a one-line change in `hardening.rs`.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 05: TLS, cookies and security headers`, push `dev`, archive this file.
