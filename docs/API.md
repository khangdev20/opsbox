# opsbox API contract (draft)

Every function exported by `web/js/api.js` maps 1:1 to one REST endpoint (JSON, cookie session, same origin). `MODE = "mock"` serves in-memory demo data; the Rust backend replaces the module body only, pages stay unchanged. Terminal uses a WebSocket carrying PTY bytes.

| Mock function | Endpoint |
|---|---|
| `tick()` | to be assigned |
| `getHost()` | `GET /api/v1/host` (implemented in Phase 0; the only real call in `MODE = 'http'`) |
| `getMetrics()` | to be assigned |
| `getAttention()` | to be assigned |
| `listContainers()` | to be assigned |
| `listDocker(kind)` | to be assigned |
| `containerAction(id, action)` | to be assigned |
| `containerLogs(id)` | to be assigned |
| `getProject()` | to be assigned |
| `getRepos()` | to be assigned |
| `createService(cfg)` | to be assigned |
| `redeploy(svc)` | to be assigned |
| `rollback(svc, deployId)` | to be assigned |
| `buildLog(svc, deploy)` | to be assigned |
| `getUpdates()` | to be assigned |
| `installUpdates(which)` | to be assigned |
| `setAutoUpdates(on)` | to be assigned |
| `reboot()` | to be assigned |
| `getUnits()` | to be assigned |
| `unitAction(name, action)` | to be assigned |
| `getJournal({ unit = '', prio = 6, q = '' } = {})` | to be assigned |
| `getJobs()` | to be assigned |
| `runJob(id)` | to be assigned |
| `addJob(job)` | to be assigned |
| `getFirewall()` | to be assigned |
| `addRule(r)` | to be assigned |
| `removeRule(id)` | to be assigned |
| `openSsh()` | to be assigned |
| `scanCleanup()` | to be assigned |
| `runCleanup(ids)` | to be assigned |
| `setAutoCleanup(on)` | to be assigned |
| `listDir(path)` | to be assigned |
| `readFile(path)` | to be assigned |
| `writeFile(path, content)` | to be assigned |
| `chmod(path, mode)` | to be assigned |
| `mkdir(path)` | to be assigned |
| `removePaths(paths)` | to be assigned |
| `upload(dir, file)` | to be assigned |
| `getSecurity()` | to be assigned |
| `toggleCheck(id)` | to be assigned |
| `setAllowIps(v)` | to be assigned |
| `inviteUser(name)` | to be assigned |
| `signOutAll()` | to be assigned |
| `getRecovery()` | to be assigned |
| `simulateFailure()` | to be assigned |
| `recoveryAction(a)` | to be assigned |
| `termExec(host, cwd, line)` | to be assigned |
| `getReclaimable()` | to be assigned |
| `getAudit(n = 5)` | to be assigned |
| `addVar(svc, k, v)` | to be assigned |
| `removeVar(svc, k)` | to be assigned |
| `removeService(svc)` | to be assigned |
| `getBackups()` | to be assigned |
| `runBackup(id)` | to be assigned |
| `restoreBackup(id, mode)` | to be assigned |
| `addDestination(d)` | to be assigned |
| `getAlerts()` | to be assigned |
| `setRule(id, on)` | to be assigned |
| `testAlert(channel)` | to be assigned |
| `connectChannel(id, to)` | to be assigned |
| `silenceAlert()` | to be assigned |
| `setQuiet(q)` | to be assigned |
| `addTemplate(id, cfg)` | to be assigned |
| `deployCompose(text)` | to be assigned |
| `getAccess()` | to be assigned |
| `addKey(user, pub)` | to be assigned |
| `removeKey(id)` | to be assigned |
| `lockUser(name, lock)` | to be assigned |
| `rotateOutKey(id)` | to be assigned |
| `getSettings()` | to be assigned |
| `saveSettings(v)` | to be assigned |
| `createToken(name, can)` | to be assigned |
| `revokeToken(id)` | to be assigned |
| `setSwap(v)` | to be assigned |
| `growFs()` | to be assigned |
| `setOpsboxAutoUpdate(on)` | to be assigned |
| `disconnectGithub()` | to be assigned |
| `exportConfig()` | to be assigned |
