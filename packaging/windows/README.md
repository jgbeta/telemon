# Windows Service Installer

The Windows packaging path is currently the PowerShell service installer. MSI packaging is still deferred, but the installed exporter now has real Windows baseline and inventory collectors plus optional LibreHardwareMonitor HTTP temperatures.

Default paths:

```text
Binary:  C:\Program Files\Telemon\telemon-exporter.exe
Config:  C:\ProgramData\Telemon\exporter.yml
Service: TelemonExporter
Port:    9185/tcp
```

The first install copies `packaging\windows\config.default.yml` to `C:\ProgramData\Telemon\exporter.yml`. Later installs preserve the existing config.

Build the exporter first:

```powershell
cargo build --release -p telemon-exporter
```

Install from an Administrator PowerShell:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe
```

Optionally add a source-restricted firewall rule for TCP `9185`:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -PrometheusServerIp <monitoring-server-ip>
```

Registry enrollment also accepts `-RegistryServer`, `-EnrollmentToken`, `-UserName`, `-DeviceName`, optional `-AdvertisedAddr`, and optional `-MachineUuid` for dual-boot or multi-OS physical-machine grouping.

The older broad firewall option is still available:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -AddFirewallRule
```

LibreHardwareMonitor CPU temperatures are optional. Enable LibreHardwareMonitor's Remote Web Server, then validate provider availability with:

```powershell
Invoke-RestMethod http://127.0.0.1:8085/data.json
```

If LibreHardwareMonitor is not running or the endpoint is unavailable, `windows_lhm_http` reports unsupported/down and the exporter keeps running. The older `windows_lhm_wmi` collector remains experimental and disabled by default.

If the default `LocalService` identity cannot access required Windows APIs on a test host, retry with:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -ServiceAccount LocalSystem
```

Check:

```powershell
Get-Service TelemonExporter
Invoke-WebRequest http://127.0.0.1:9185/healthz
Invoke-WebRequest http://127.0.0.1:9185/metrics
Invoke-WebRequest http://127.0.0.1:9185/metrics/static
```

Uninstall:

```powershell
.\packaging\windows\uninstall-service.ps1
```

The uninstall script removes `Telemon Exporter 9185*` firewall rules and preserves `C:\ProgramData\Telemon\exporter.yml`. Use `-PreserveFirewall` for service repair or migration, and `-RemoveFiles` for a full local reset.
For full reset and reinstall cleanup commands, see `..\..\docs\uninstall.md`.
