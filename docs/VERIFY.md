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
