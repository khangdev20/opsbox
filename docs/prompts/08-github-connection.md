# 08. GitHub connection (Phase 3)

Spec: 7.5 steps 1-2. Crate: `opsbox-deploy`.

Do:
- Connect GitHub with a GitHub App created through the manifest flow from Settings (so each user owns their app; no shared opsbox app). Store app id, private key and webhook secret encrypted.
- Repo picker real (`getRepos`): installed repositories, search, detected build method.
- Webhook endpoint for push events, HMAC-verified, reachable through the panel domain from prompt 05 (document the requirement if no domain is set yet).
- Fallbacks: public repo URL, and deploy key for a single private repo.
- Settings "GitHub" section real (connected account, manage access link, disconnect).

Out of scope: building and running (prompt 09).
