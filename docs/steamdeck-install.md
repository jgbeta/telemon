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

The bundled installer can find the bundled `telemon-exporter` binary automatically.

## Verify

On the Steam Deck:

```bash
systemctl --user status telemon-exporter.service --no-pager
journalctl --user -u telemon-exporter.service -n 100 --no-pager
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/readyz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
```

From the Prometheus host, validate that the Deck is reachable on the LAN:

```bash
curl http://<steam-deck-lan-ip>:9185/metrics
curl http://<steam-deck-lan-ip>:9185/metrics/static
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

This phase only installs the existing native Linux exporter as a Steam Deck user service. It does not add Steam Deck-specific collectors, Gamescope game-state sampling overrides, `/fps`, fan control, TDP control, or MangoHUD-style frame timing metrics yet.
