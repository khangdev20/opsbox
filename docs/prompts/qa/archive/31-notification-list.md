# QA 31. Notification list: stray commas, stale alerts, restarting timestamps

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

Seen in the bell panel:
1. A "," character shows between notification rows.
2. Three alerts say "Container greenglish-dev-web-1 (api-1, worker-1) is stopped or restarting" 8 hours ago, while Containers shows all three `running 22 h`. Alerts for a condition that is no longer true must resolve themselves.
3. "3 security updates are waiting" shows "36 s ago" even though updates have been waiting for days: the time looks reset on every poll.

## Check
1. Commas: read `renderNotifItems` and the `html` template in `web/js/shell.js` (about lines 107 and 170). A JS array put into a template string is joined with commas; find where an array is stringified instead of joined or flattened by `html`.
2. Stale alerts: in `crates/opsbox-api/src/alerts.rs` and the alert evaluator, find how "container stopped or restarting" is evaluated and resolved. Reproduce on `opsbox-vm`: stop a container, see the alert; start it, wait one interval: the alert must resolve and the bell item must change to resolved or disappear.
3. Timestamps: find where the "security updates waiting" notification is created. It must be keyed (`kind + subject`) and updated in place; its time is the first time it was raised, with `updated_at` separate. Check the store for a new row or a new `created_at` per poll.

## Fix
- Commas: join array output (`.join('')`) or fix `html` to flatten arrays; add a test that renders three items and asserts no `,` text node.
- Stale alert: if the resolve step is missing or compares the wrong state, fix it in one place and add a test (condition true, then false, expects resolved). If the evaluator needs redesign, write it up under "Found, not fixed" with the file and line.
- Timestamp: upsert by key and keep the original `created_at`; add a test (raise twice, one row, same created_at).

## Pass when
No commas; the three container alerts resolve after the containers run (or are written up with the cause); the updates notification keeps its original time across polls.
