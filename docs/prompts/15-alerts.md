# 15. Alerts (Phase 5)

Spec: 7.13. Crate: `opsbox-alerts`.

Do:
- Rule engine over metrics and events with the state machine (ok, pending, firing, resolved), dedupe, repeat every 4 h, silence, quiet hours (warnings wait, critical always sends).
- Default rules from the Alerts screen: disk, CPU, memory, unit or container down, deploy failed, backup failed or missed, certificate expiring, security updates waiting, repeated failed sign-ins, emergency SSH open.
- Channels: email (SMTP), Slack webhook, Telegram bot, generic webhook (HMAC-signed). Secrets encrypted. "Send test".
- Wire the Alerts page; Overview attention links to the matching alert.

Verify: fill the VM disk to trigger the disk rule; receive it on one channel; resolve it and get the resolved message.
