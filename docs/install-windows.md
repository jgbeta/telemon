# Windows Install

Windows support currently uses the PowerShell service installer. MSI packaging is still deferred. The Windows exporter emits real baseline OS metrics, optional NVIDIA NVML GPU metrics, and optional CPU/motherboard/storage temperatures through the LibreHardwareMonitor local HTTP endpoint.

## Build

Build a Windows release binary on Windows:

```powershell
cargo build --release
```

## Install

For UUID enrollment, start the central registry stack first and confirm the client host can reach `http://<server-ip>:9186/healthz`. The service retries registration if the registry is temporarily unavailable, but first install is easiest to verify with the registry online.

Run from Administrator PowerShell:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe
```

The installer creates `C:\ProgramData\Telemon\exporter.yml` from `packaging\windows\config.default.yml` on first install and preserves it on later runs. The default Windows config disables Linux hwmon, enables Windows baseline/inventory collectors, and leaves NVIDIA NVML plus `windows_lhm_http` enabled as optional collectors. Missing optional providers are non-fatal.

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

If `-PrometheusServerIp` is omitted, the installer derives the scrape source IP from `-RegistryServer` and adds a source-restricted inbound firewall rule. Omit `-AdvertisedAddr` to let the registry use the connection source IP as the Prometheus scrape target. Use `-MachineUuid` when dual-boot or multi-OS installs should share one physical-machine identity.

The older broad firewall option is still available:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -AddFirewallRule
```

If Windows API access fails under the default service identity, test with the explicit escape hatch:

```powershell
.\packaging\windows\install-service.ps1 `
  -BinaryPath .\target\release\telemon-exporter.exe `
  -ServiceAccount LocalSystem
```

Keep `LocalService` as the default unless testing shows the host needs broader permissions.

## Optional CPU Temperatures

Windows CPU/core temperatures require LibreHardwareMonitor with its Remote Web Server enabled. Generic Windows temperature WMI classes are not used as production CPU temperature sources. The older `windows_lhm_wmi` collector remains experimental and disabled by default because some LibreHardwareMonitor builds do not publish a WMI namespace.

Before expecting Telemon CPU temperatures, validate the local LibreHardwareMonitor endpoint from PowerShell:

```powershell
Invoke-RestMethod http://127.0.0.1:8085/data.json
```

If this returns LibreHardwareMonitor JSON with AMD Ryzen package/core temperature nodes, Telemon should emit them as `hardware_temperature_celsius{source="windows_lhm_http",component="cpu",...}`. If LibreHardwareMonitor is not running or the Remote Web Server is disabled, Telemon reports `exporter_collector_supported{collector="windows_lhm_http"} 0` and continues running.

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

Expected Windows MVP metrics include uptime, total CPU usage after the second collection cycle, memory bytes, fixed local filesystem bytes, network byte counters, Windows OS info, CPU info, and computer-system info. NVIDIA GPU metrics appear on Windows machines where `nvml.dll` is available. CPU/motherboard/storage temperatures appear when LibreHardwareMonitor is running with its Remote Web Server enabled.

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

The uninstall script preserves `C:\ProgramData\Telemon\exporter.yml`. It removes firewall rules named `Telemon Exporter 9185*`.
