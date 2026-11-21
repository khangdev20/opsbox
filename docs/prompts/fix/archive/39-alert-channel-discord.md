# Fix 39. Add Discord as an alert channel

Follow `docs/prompts/fix/README.md`. Small feature: copy how the Slack channel is built (`rg -n -i 'slack' crates web/js docs`) and add Discord next to it.

## Goal
Alerts page, Channels table: a new row "Discord" with Connect, like Slack. The user pastes a Discord webhook URL; alerts post there.

## Steps
1. Test first (unit, with a fake HTTP server): `POST` of a test alert to a Discord channel sends JSON `{ "embeds": [{ title, description, color, timestamp, footer: host name }] }`, colour by severity (critical red, warning amber, info blue); content longer than the limits (title 256, description 4096) is cut with an ellipsis; a 429 reply waits `retry_after` once, then reports failure.
2. Validate the URL when saving: `https`, host `discord.com` or `discordapp.com`, path `/api/webhooks/<id>/<token>`. Anything else is refused with a clear error (this also blocks using the field to reach internal addresses).
3. Store the URL as a sealed secret, the same way other channel secrets are stored; the table and API never show it again (show "…/<last 4 of token>").
4. Add the "Send test message" button and a status (off / ok / last error), like the other channels. Add the channel kind to the proto, API route, panel row and Connect dialog (one field: Webhook URL, with a one-line help: Server settings, Integrations, Webhooks, Copy URL).
5. Update `docs/` where channels are listed (and `docs/schemas/` if the export includes channels): export keeps the URL only inside the sealed secrets.

## Done when
The tests pass, a pasted valid webhook receives the test message on a real Discord server (owner check, add steps under "Owner checks" in `docs/QA.md`), and invalid URLs are refused.

Commit `fix 39: discord alert channel`, push `dev`, tag as usual, archive this file.
