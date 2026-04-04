# bdstorage on WSL (Ubuntu)

These files target **Ubuntu inside WSL2** with **systemd** enabled, using **per-user** units so you do not need `sudo` to manage the service.

## 1. Enable systemd (once per distro)

Inside Ubuntu (WSL), either edit `/etc/wsl.conf` or merge the snippet from `wsl.conf.example`:

```ini
[boot]
systemd=true
```

From **Windows** (PowerShell or CMD):

```text
wsl --shutdown
```

Reopen your Ubuntu app so WSL picks up the change. Confirm:

```bash
systemctl is-system-running
```

## 2. Build bdstorage

From your clone (Linux path, e.g. under `/home/you/...`):

```bash
cargo build --release
```

## 3. Install user units

```bash
chmod +x contrib/wsl/install-wsl-user-units.sh
./contrib/wsl/install-wsl-user-units.sh
```

Optional flags:

```bash
./contrib/wsl/install-wsl-user-units.sh \
  --binary "$HOME/bdstorage/target/release/bdstorage" \
  --target /mnt/c/Users/You/Documents \
  --interval 7200
```

This creates `~/.config/bdstorage/env` (if missing) and copies units to `~/.config/systemd/user/`. Edit `BDSTORAGE_TARGET` there to point at the directory tree you want deduplicated.

State and vault live under **`$HOME/.imprint/`** (see `default_db_path` in `src/state.rs`).

## 4. Start the daemon or timer

**Daemon** (same process repeatedly spawning `dedupe`):

```bash
systemctl --user enable --now bdstorage-dedupe.service
journalctl --user -u bdstorage-dedupe.service -f
```

**Timer** (daily one-shot `dedupe`, no long-running loop):

```bash
systemctl --user enable --now bdstorage-dedupe.timer
systemctl --user list-timers bdstorage-dedupe.timer
```

Do not enable both at once on the same target unless you intend overlapping runs.

## 5. Optional: user lingering

So user units can start after WSL boots without an interactive login:

```bash
loginctl enable-linger "$USER"
```

Behavior depends on WSL version and how you start the distro; this is still useful on many setups.

## Files

| File | Purpose |
|------|---------|
| `bdstorage-dedupe.service` | `bdstorage daemon …` under `systemctl --user` |
| `bdstorage-dedupe-once.service` | One `bdstorage dedupe` run |
| `bdstorage-dedupe.timer` | Daily trigger for the oneshot unit |
| `env.example` | Reference for `~/.config/bdstorage/env` |
| `wsl.conf.example` | Enable systemd in WSL |

System-wide units (optional, for non-WSL Linux) remain under `contrib/systemd/`.
