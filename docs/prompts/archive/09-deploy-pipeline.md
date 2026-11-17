# 09. Deploy pipeline (Phase 3)

Spec: 7.5 steps 3-10, 7.1 tasks, 6.4 (services, deploys, vars). Uses prompts 07 and 08.

Do:
- Services, deploys, service_vars tables. Projects and one environment ("production") for now.
- Clone at commit, detect (Dockerfile, else static site; Nixpacks optional per DECISIONS), BuildKit build with per-service cache, secrets via BuildKit secret mounts, timeouts.
- Start new container on the project network with env vars and resource limits; HTTP or TCP health check; switch the proxy route; drain old; keep last 5 images; rollback without rebuild.
- Any failure before the switch leaves the old version live and marks the deploy failed.
- UI real: New service, Configure, Apps graph and deploys tab, build log streaming, variables, redeploy, rollback, delete service. Deployer role jailed to its own service directories; API tokens scoped `deploy:<service>`.

Verify: deploy a sample repo with a Dockerfile, push a change, watch auto-deploy; push a broken build and confirm the old version keeps serving.
