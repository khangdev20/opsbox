# Verifying each milestone in the VM

Steps for a Multipass VM named `opsbox-vm` with the repo mounted at `/home/ubuntu/opsbox`. Each milestone adds a section; the preamble is the same every time.

## Preamble: build, reinstall, open the UI

On the Mac:

```sh
multipass shell opsbox-vm
```

In the VM:

```sh
cd ~/opsbox && git pull          # if the mount is not live
export CARGO_TARGET_DIR=~/target
cargo build --release
sudo sh install.sh --dev --binary ~/target/release/opsbox
opsbox status                    # both lines "ok"
```

On the Mac, tunnel the port (the server listens on loopback only):

```sh
ssh -L 8443:127.0.0.1:8443 ubuntu@$(multipass info opsbox-vm --format csv | tail -1 | cut -d, -f3)
# or: multipass exec opsbox-vm -- ...  if you prefer a port forward of your own
```

Then open <http://127.0.0.1:8443/?mode=http>. Without `?mode=http` the UI runs on built-in sample data.

---

## M1. Honest UI in http mode

1. Open `http://127.0.0.1:8443/?mode=http` and go through the sidebar.
2. Every page except the ones listed as real shows a small amber badge in its header: **sample data**, or **partly sample data** when the page mixes real and sample data (Overview: the header line is real, the cards are not yet). Hover the badge to see which functions are still sample.
3. The top bar shows this machine's real hostname (not `srv-sgn-01 / shop-platform / production`), and the status bar says **sample metrics** until M3.
4. Open `http://127.0.0.1:8443/?mode=mock`: no badges, the old demo data, `mock data` in the status bar.

Wiring a function later is a one-line change: add its name to `REAL` in `web/js/api.js`. The badge on every page that depends only on real functions then disappears by itself.

---

## M2. Auth and first run

Reinstall as in the preamble. The installer now ends by printing the first-run setup block:

```
opsbox first-run setup
  setup url    http://127.0.0.1:8443/#/setup
  setup token  XXXX-XXXX-XXXX   (single use, valid for 1 hour)
```

The same block is in the journal (`sudo journalctl -u opsbox | grep -A3 "first-run setup"`) and `sudo opsbox setup-token` prints a fresh one at any time while no admin exists.

**Setup in the UI** (open `http://127.0.0.1:8443/?mode=http` through the tunnel):

1. You land on the **Set up** page without signing in. A wrong token shows "That token is not right…"; the right one (lower case is fine) goes to step 2.
2. Create the admin: a short password, or two passwords that differ, are refused with a sentence. Use 12+ characters.
3. Step 3 shows a **QR code** (drawn in the page, no network request) and the key. Scan it with an authenticator app. A wrong code is refused; the right one signs you in and opens Overview.
4. Open `#/setup` again: you are sent back to the app. From a private window the page goes to sign-in. Setup is closed for good (`curl -i -X POST http://127.0.0.1:8443/api/v1/setup/claim -H 'Content-Type: application/json' -d '{"token":"AAAA-BBBB-CCCC"}'` answers 404).

**Sign-in and sessions**

5. Click **admin · sign out** (top right): you land on Sign in. Sign in with password + a code. The same code cannot be used twice: sign out and in again within the same 30 seconds with the same code, and it is refused. Wait for the next code.
6. Five wrong attempts in a row: "Too many attempts. Try again in 30 seconds." (and a longer wait after more).
7. In the browser dev tools (Application → Cookies): `opsbox_session` is **HttpOnly** and **SameSite=Strict**; `opsbox_csrf` is readable. `Secure` appears only behind an HTTPS proxy.
8. Security → Sign out all sessions ends every session, including this one.

**Command line** (in the VM):

```sh
sudo opsbox status                          # still ok / ok
sudo opsbox audit verify                    # "Audit log is intact: N entries"
sudo opsbox user unlock admin               # clears a lockout
sudo opsbox user reset-2fa admin            # forget the authenticator, end sessions
# Sign in with the password only: you are taken to a page that sets up a new authenticator.
echo 'a new long passphrase' | sudo opsbox user password admin
sudo opsbox setup-token                     # refuses: an admin exists, setup is closed
```

**Check the files and the audit log**

```sh
ls -l /etc/opsbox/master.key                # -rw------- root root 32 bytes (created on first sign-up)
sudo -u opsbox sqlite3 /var/lib/opsbox/opsbox.db "select seq,actor,action,result from audit order by seq" | tail
```

You should see `setup.*`, `auth.login` (ok and denied), `auth.lockout`, `user.*` entries, and `audit verify` stays intact.

---

## M3. Metrics and Overview

Reinstall as in the preamble, sign in, and open **Overview**.

1. The amber "sample data" badge is **gone** from Overview (hover nothing: there is none), and the status bar at the bottom says **live** instead of "sample metrics". The header line still shows the real hostname, OS, kernel, uptime and core count.
2. The four cards (CPU, Memory, Disk /, Network) and the chart move every second. In the VM run `yes > /dev/null &` for a few seconds (then `kill %1`): CPU jumps, `yes` appears at the top of **Top processes**, and `opsbox` rows are marked as this pane.
3. **Disk /** shows the real used/total GB of the root filesystem and the free space. `dd if=/dev/zero of=/tmp/big bs=1M count=2000` then `rm /tmp/big` moves it.
4. **Needs attention** is real: on a machine with a pending security update it says "N security updates are waiting"; `sudo touch /run/reboot-required` adds "A reboot is needed…" (remove the file to clear it, it refreshes within five minutes); a disk over 75% adds "Disk / is at N%"; `sudo systemctl start nonexistent` does not count, but a failed unit does (`sudo systemd-run --unit=boom /bin/false` then `systemctl reset-failed boom` to clear). Updates and failed units are re-read every five minutes.
5. Dev tools: Network tab → WS shows one connection to `/api/v1/stream/metrics` delivering a message every second, and navigating between pages does not open another. Sign out and it closes.

From a shell (replace the cookie with yours from the browser, or use the UI):

```sh
curl -s http://127.0.0.1:8443/api/v1/metrics -b 'opsbox_session=…' | python3 -m json.tool | head -40
sqlite3 /var/lib/opsbox/opsbox.db "select ts,round(cpu,1),round(mem,1),net_in from metrics_minute order by ts desc limit 5"   # as the opsbox user; rows appear a minute after start
```

**Idle memory of `serve`** (the spec's target is under 40 MB). After the page has been closed for a minute:

```sh
ps -o pid,rss,cmd -C opsbox | awk 'NR==1{print} NR>1{printf "%s  %.1f MB  %s %s\n",$1,$2/1024,$3,$4}'
systemctl show opsbox -p MemoryCurrent
```

---

## M4. System

**Read this first.** None of the system tools (apt, dnf, systemd, ufw, nft) can run on the machine this was written on, so this milestone is covered by tests of everything that does not need them (parsers on real-looking output, the cron converter, the lockout-guard decision, validation, roles) and by CI. **This section is the first real run.** Take a VM snapshot before the firewall steps. Reinstall as in the preamble (the `opsbox-priv.service` unit changed: it now allows network and netlink sockets for apt and nft).

Open **System** (`#/system`). The "sample data" badge is gone.

**Updates tab**

1. Compare with `apt list --upgradable` in the VM: same packages, the **security** tag on those from `*-security`, the running kernel, and "checked HH:MM" (the time of the last `apt update`).
2. **Install security updates**: the button stays busy for a while (it runs `apt-get update` then installs only the security packages). Watch it: `sudo journalctl -u opsbox-priv -f`. It finishes with "Installed N updates." Running it twice at once is refused ("already running").
3. If a kernel was installed (or `/run/reboot-required` exists) the **Reboot** callout appears. **Reboot…** asks, then schedules in one minute: a red banner "A reboot is scheduled for HH:MM" with **Cancel reboot** appears (`shutdown -c` equivalent). Cancel it: the banner goes. (`ls /run/systemd/shutdown/scheduled` exists while scheduled.)
4. The **automatic security updates** switch writes `/etc/apt/apt.conf.d/20auto-upgrades` (check `cat` it) and installs `unattended-upgrades` if needed.

**Services tab**

5. Failed units first, then running ones, with memory. **Restart** a harmless service (`cron`); it works. **Stop** `opsbox`, `ssh`, `systemd-networkd`: each is **refused with a sentence** ("opsbox itself", "only way back in", ...). Restarting `ssh` is allowed.

**Logs tab**

6. Lines of the journal (oldest first); filter by text and priority; "unit: x ✕" appears when you came from a service. Press **Live ●**: in another shell run `logger -t demo hello` and the line appears within about two seconds, appended below. Switching tab stops it.

**Jobs tab**

7. **Add job**: schedule `*/5 * * * *`, description "demo", command `date >> /tmp/opsbox-demo.log`. Check: `systemctl list-timers | grep opsbox-job`, `cat /etc/systemd/system/opsbox-job-*.service`. **Run now** runs it (the file gets a line). **Remove** deletes both units. Try `0 2 1 * 1` (both day-of-month and day-of-week): refused with a readable sentence; also `0 2 * *`.

**Firewall tab** (Ubuntu has ufw installed but off, so this starts on the `nftables` backend)

8. The rules list is empty (no firewall is on). **Add rule** port `8080`: because this turns the firewall on, you are asked first ("This turns the firewall on…"). Confirm: a **countdown dialog** appears ("Undoing in 60 s"). Open a *second* SSH session to prove you are not locked out, then press **Yes, keep it**. `sudo nft list table inet opsbox` shows `policy drop`, SSH, the WireGuard port and 8080.
9. Remove the 8080 rule: no question (not an SSH rule).
10. **Lockout guard**: add rule `22/tcp from anywhere`, then remove it. You are warned ("lets you reach port 22 (SSH)… could lock you out"). Confirm, then *do not press Keep*: after 60 s the rule is put back by itself (watch `sudo nft list table inet opsbox`). Because you are reaching the panel through a tunnel the panel treats every removal on an SSH port as risky, which is intended.
11. **Open emergency SSH for 1 hour**: the panel asks which public address you connect from (it only sees `127.0.0.1` through a tunnel), opens SSH to it, and shows the callout. After an hour (or `sudo rm`-ing nothing: just wait, or edit `/etc/opsbox/firewall.json` `until` to the past; the janitor runs every 10 s) the entry disappears.
12. To try the ufw backend: `sudo ufw allow 22/tcp && sudo ufw enable`, reload the page: the same tab now shows `ufw` rules; add and remove work and the same guard applies.

**Audit**: every change above is in the audit log with your name and address: `sudo opsbox audit verify`; `sudo -u opsbox sqlite3 /var/lib/opsbox/opsbox.db "select seq,actor,action,target,result from audit where action like 'system.%'"`.

Not wired yet (still sample data, by design): the Settings page (hostname, time zone and swap controls). The operations and endpoints exist (`PUT /api/v1/system/swap|hostname|timezone`); the page is wired in M7 together with real settings data, because it is prefilled from mock values today.
