# Packaging

Packaging turns the native exporter binary into OS-managed services.

Expected local build output:

```text
dist/
  current/
    telemon-exporter-v<version>-<platform>-<arch>.tar.gz
    SHA256SUMS
    telemon-exporter_<version>_<platform>_<arch>
  linux/
    telemon-exporter_<version>_<arch>.deb
```

`dist/` is generated build output and should not be committed.

The versioned `tar.gz` bundle is the canonical release artifact for fallback
installs and unsupported Linux variants. The raw binary remains in `dist/current`
only for local packaging compatibility.

Phase 3 package targets:

- Linux: `.deb` package with systemd service.
- Windows: service skeleton and PowerShell install/uninstall scripts.
- macOS: LaunchDaemon skeleton and shell install/uninstall scripts.

Installers can optionally add a source-restricted firewall allow rule for the
Prometheus server IP so Prometheus can scrape TCP `9185`.

RPM, MSI, package signing, Windows hardware collectors, and macOS thermal collection are deferred to later phases.
