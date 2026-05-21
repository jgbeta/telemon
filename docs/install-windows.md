# Windows Install

Phase 3 supports a Windows service skeleton. Windows MSI packaging and Windows temperature collection are deferred to Phase 5.

## Build

Build a Windows release binary on Windows:

```powershell
cargo build --release
```

## Install

For UUID enrollment, start the central registry stack first and confirm the
client host can reach `http://<server-ip>:9186/healthz`. The service retries
registration if the registry is temporarily unavailable, but first install is
easiest to verify with the registry online.

Run from Administrator PowerShell:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe
```

To add a source-restricted inbound firewall rule for the Prometheus server:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -PrometheusServerIp <monitoring-server-ip>
```

This allows TCP `9185` only from the specified Prometheus server IP.

To enroll the device with the registry during install:

```powershell
.\packaging\windows\install-service.ps1 `
  -BinaryPath .\target\release\telemon-exporter.exe `
  -RegistryServer registry.example.local:9186 `
  -EnrollmentToken change-me `
  -UserName example-user `
  -DeviceName gaming-pc `
  -AdvertisedAddr exporter.example.local `
  -MachineUuid <shared-machine-uuid-if-dual-boot>
```

If `-PrometheusServerIp` is omitted, the installer derives the scrape source IP
from `-RegistryServer` and adds a source-restricted inbound firewall rule.
Omit `-AdvertisedAddr` to let the registry use the connection source IP as the
Prometheus scrape target. Use `-MachineUuid` when dual-boot or multi-OS installs
should share one physical-machine identity.

The older broad firewall option is still available:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -AddFirewallRule
```

Installed paths:

```text
Binary:  C:\Program Files\Telemon\telemon-exporter.exe
Config:  C:\ProgramData\Telemon\exporter.yml
State:   C:\ProgramData\Telemon\state\device-id
Service: TelemonExporter
```

## Check

```powershell
Get-Service TelemonExporter
Invoke-WebRequest http://127.0.0.1:9185/healthz
Invoke-WebRequest http://127.0.0.1:9185/metrics
Invoke-WebRequest http://127.0.0.1:9185/metrics/static
Invoke-WebRequest http://<exporter-lan-ip>:9185/metrics
Invoke-WebRequest http://<exporter-lan-ip>:9185/metrics/static
```

For registry enrollment, check the registry from the monitoring server:

```powershell
curl.exe http://<server-ip>:9186/prometheus/sd
```

From the Prometheus server:

```powershell
curl.exe -v --connect-timeout 3 http://<exporter-lan-ip>:9185/metrics
```

## Uninstall

```powershell
.\packaging\windows\uninstall-service.ps1
```

The uninstall script preserves `C:\ProgramData\Telemon\exporter.yml`.
It removes firewall rules named `Telemon Exporter 9185*`.
