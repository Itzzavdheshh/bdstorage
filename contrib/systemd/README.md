# bdstorage on bare metal (systemd)

Example: **Ubuntu** (22.04/24.04 or similar; any distro with systemd works with the same unit layout).

Units assume:

- Binary: `/usr/local/bin/bdstorage`
- Config: `/etc/bdstorage.env` (not world-readable; holds `HOME`, `BDSTORAGE_TARGET`, `BDSTORAGE_INTERVAL_SECS`)
- Service account: `bdstorage` with home `/var/lib/bdstorage` (state and vault live in **`~/.imprint/`** for that user)

## 1. Build and install the binary

```bash
cd /path/to/bdstorage
cargo build --release
sudo install -m755 target/release/bdstorage /usr/local/bin/bdstorage
```

## 2. Service user and home

```bash
sudo useradd --system --home-dir /var/lib/bdstorage --create-home --shell /usr/sbin/nologin bdstorage
```

## 3. Permissions on the tree to dedupe

The `bdstorage` user must be able to **read** all files under `BDSTORAGE_TARGET` and **modify** them when deduplicating (reflinks / hardlinks). Typical options:

- **Own the tree:**  
  `sudo chown -R bdstorage:bdstorage /srv/data`
- **Or** use ACLs / group membership so `bdstorage` can access existing data (adjust to your layout).

## 4. Environment file

```bash
sudo cp contrib/systemd/bdstorage.env.example /etc/bdstorage.env
sudo chmod 600 /etc/bdstorage.env
sudo editor /etc/bdstorage.env   # set BDSTORAGE_TARGET (and HOME if you changed the account home)
```

## 5. Install unit files

```bash
sudo install -m644 contrib/systemd/bdstorage-dedupe.service /etc/systemd/system/
sudo install -m644 contrib/systemd/bdstorage-dedupe-once.service /etc/systemd/system/
sudo install -m644 contrib/systemd/bdstorage-dedupe.timer /etc/systemd/system/
sudo systemctl daemon-reload
```

## 6. Choose daemon **or** timer

**Long-running daemon** (sleeps between runs; interval from `BDSTORAGE_INTERVAL_SECS`):

```bash
sudo systemctl enable --now bdstorage-dedupe.service
sudo systemctl status bdstorage-dedupe.service
sudo journalctl -u bdstorage-dedupe.service -f
```

**Timer only** (e.g. daily one-shot; no background loop):

```bash
sudo systemctl disable --now bdstorage-dedupe.service 2>/dev/null || true
sudo systemctl enable --now bdstorage-dedupe.timer
sudo systemctl list-timers bdstorage-dedupe.timer
```

Do not enable both the daemon and the timer against the same tree unless overlapping runs are intentional.

## 7. Manual test

```bash
sudo -u bdstorage env HOME=/var/lib/bdstorage /usr/local/bin/bdstorage dedupe /srv/data -n
```

(remove `-n` when satisfied)

## Files in this directory

| File | Purpose |
|------|---------|
| `bdstorage-dedupe.service` | System service: `bdstorage daemon …` |
| `bdstorage-dedupe-once.service` | Oneshot `bdstorage dedupe` |
| `bdstorage-dedupe.timer` | Runs the oneshot on a schedule |
| `bdstorage.env.example` | Template for `/etc/bdstorage.env` |

For **WSL** and per-user units (no sudo for `systemctl --user`), see `contrib/wsl/`.
