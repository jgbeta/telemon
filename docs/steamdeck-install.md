# Steam Deck Install

This is the Steam Deck compatibility install path for `telemon-exporter`. It is a user-space bootstrapper for SteamOS and does not replace the normal Linux package flow.

Use this path when you want to run the native exporter on a Steam Deck without Cargo, `pacman`, sudo install paths, or disabling the SteamOS read-only filesystem.

## What It Installs

The installer writes only user-owned persistent paths for the `deck` user:

```text
Binary:  /home/deck/.local/bin/telemon-exporter
Config:  /home/deck/.config/telemon/exporter.yml
State:   /home/deck/.local/state/telemon/exporter/
Service: /home/deck/.config/systemd/user/telemon-exporter.service
```

It does not install to `/usr/bin`, `/usr/local/bin`, `/etc/systemd/system`, `/etc/telemon`, or `/var/lib/telemon`.

## Prerequisites

Build or download a Linux x86_64 `telemon-exporter` release bundle before running the installer on the Deck. The Deck install path does not build Rust code on the target device.

From a development machine:

```bash
bash scripts/build-release.sh
```

Copy the generated exporter archive to the Steam Deck, for example:

```text
dist/current/telemon-exporter-v<version>-linux-x86_64.tar.gz
```

The Telemon core/registry should already be reachable from the Deck if you want automatic registration.

## Install From A Cloned Repo

Run this from Desktop Mode as the `deck` user:

```bash
bash scripts/install-steamdeck.sh \
  --artifact dist/current/telemon-exporter-v<version>-linux-x86_64.tar.gz \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name steam-deck
```

Optional grouping for a device that already has a Telemon machine identity:

```bash
bash scripts/install-steamdeck.sh \
  --artifact dist/current/telemon-exporter-v<version>-linux-x86_64.tar.gz \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name steam-deck \
  --machine-uuid <existing-machine-uuid>
```

## Install From A Release Bundle

If the Steam Deck release bundle has already been extracted:

```bash
tar -xzf telemon-exporter-v<version>-linux-x86_64.tar.gz
cd telemon-exporter-v<version>-linux-x86_64
bash install-steamdeck.sh \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name steam-deck
```

To test experimental FPS/frame-time telemetry, add `--enable-fps` and regenerate
the config with `--force-config` if a previous config already exists:

```bash
bash install-steamdeck.sh \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name steam-deck \
  --enable-fps \
  --force-config
```

The bundled installer can find the bundled `telemon-exporter` binary automatically.

## Enabled Deck Telemetry

The generated Steam Deck profile enables Linux `hwmon`, `/proc` system metrics, `linux_power_supply`, `linux_amdgpu`, and optional `steam_deck_game_state` sampling detection. The AMDGPU collector reads Steam Deck/APU `gpu_metrics` when available for CPU temperature, APU power, GPU clocks, and throttle flags. Gamescope detection is non-fatal; Telemon reads Gamescope X11 atoms first, can discover Steam's `DISPLAY`/`XAUTHORITY`, and falls back to Desktop active-window or Steam process-tree detection before returning to temperature-based adaptive sampling.

When `--enable-fps` is used, the profile also enables the gated `/fps` endpoint
and prefers MangoHud-compatible log tailing before the direct Gamescope/MangoApp
queue. The default source order is `mangohud_log`, then `gamescope_mangoapp`.
For normal use with MangoHud enabled, configure MangoHud logging and let Telemon
tail the newest CSV log from the configured paths or common user log locations.
This avoids consuming frame messages from the same queue MangoHud uses.

Telemon exports rolling aggregate FPS, frame-time, 1% low, 0.1% low, 1% high,
and pacing jitter metrics. It does not export raw per-frame samples. Game names
are resolved locally from Steam `appmanifest_<appid>.acf` files when available.

The direct `gamescope_mangoapp` source remains useful for diagnostics and setups
without MangoHud logging. It reads the System V message queue used by
Gamescope/MangoApp, including the SteamOS compatibility fallback where traffic
appears on the failed-`ftok("mangoapp", 65)` queue key shown by `ipcs -q` as
`0xffffffff`. Queue reads are destructive, so the Steam Deck profile sets
`allow_destructive_read: false` by default. If a `mangoapp` or `MangoHud` process
is detected, the direct source reports `queue="blocked_competing_consumer"`
instead of stealing samples. Only set `allow_destructive_read: true` when you are
intentionally running Telemon as the exclusive queue consumer.

## Verify

On the Steam Deck:

```bash
systemctl --user status telemon-exporter.service --no-pager
journalctl --user -u telemon-exporter.service -n 100 --no-pager
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/readyz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
curl http://127.0.0.1:9185/fps
```

From the Prometheus host, validate that the Deck is reachable on the LAN:

```bash
curl http://<steam-deck-lan-ip>:9185/metrics
curl http://<steam-deck-lan-ip>:9185/metrics/static
curl http://<steam-deck-lan-ip>:9185/fps
```

The installer does not change SteamOS firewall or router settings. If the local curl works but the Prometheus host cannot scrape, debug LAN reachability to TCP port `9185`.

## Re-running

Re-running the installer updates the binary and user service. Existing config and state are preserved by default.

Use `--force-config` only when you want to regenerate the config from installer arguments. The old config is backed up with a timestamp first.

Use `--enable-linger` if you want the user service to be allowed to start independently of the interactive user session:

```bash
bash scripts/install-steamdeck.sh --artifact telemon-exporter-v<version>-linux-x86_64.tar.gz --enable-linger
```

That option may prompt for sudo because it runs `sudo loginctl enable-linger deck`.

## Uninstall

Stop and remove the user service and binary:

```bash
systemctl --user disable --now telemon-exporter.service
rm -f ~/.config/systemd/user/telemon-exporter.service
systemctl --user daemon-reload
rm -f ~/.local/bin/telemon-exporter
```

Remove config and state only when you intentionally want a clean re-enrollment:

```bash
rm -rf ~/.config/telemon ~/.local/state/telemon/exporter
```

## Current Limits

FPS telemetry is experimental. The preferred `mangohud_log` source requires MangoHud to write CSV logs; if `game_frame_source_supported` is `1` and `game_frame_source_up` is `0`, Telemon found a candidate log or queue but has not recently received valid frame samples. For `source="mangohud_log"`, check MangoHud logging and `mangohud_log.paths`. For `source="gamescope_mangoapp"`, check `ipcs -q`, the `queue` label, and whether direct reads are blocked by a competing MangoHud/mangoapp consumer. Normal hardware telemetry and game-state sampling can still be working correctly. Fan control and TDP control are not implemented.
