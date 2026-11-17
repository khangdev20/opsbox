# 07. Docker and Containers page (Phase 3)

Spec: 7.4, 7.11 (Docker scanners). Crate: `opsbox-docker`.

Do:
- Only priv talks to `/var/run/docker.sock` (bollard). PrivOps for list, inspect, start, stop, restart, remove, logs (follow), stats, exec.
- If Docker is missing, the Containers page offers "Install Docker" (distro packages or Docker's signed repo; never the convenience script).
- Containers page real: containers, images, volumes, networks; logs streaming; actions with confirmations. Unmanaged containers shown and controllable, never auto-deleted.
- Cleanup: Docker scanners (unused images, stopped containers, build cache; dangling volumes opt-in and flagged).
- Overview: container stats in top processes and attention (restarting containers).

Verify: install Docker from the UI on the VM, run `docker run -d nginx` from the terminal page, see it, stop it, read logs.
