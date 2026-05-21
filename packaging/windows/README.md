# Windows Service Skeleton

Phase 3 adds Windows service installation and management scripts without Windows hardware collectors.

Default paths:

```text
Binary:  C:\Program Files\Telemon\telemon-exporter.exe
Config:  C:\ProgramData\Telemon\exporter.yml
Service: TelemonExporter
Port:    9185/tcp
```

Install from an Administrator PowerShell:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe
```

Optionally add a local firewall rule for TCP 9185:

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -PrometheusServerIp <monitoring-server-ip>
```

This allows TCP `9185` only from the Prometheus server. The older broad firewall option is still available:

Registry enrollment also accepts `-RegistryServer`, `-EnrollmentToken`,
`-UserName`, `-DeviceName`, optional `-AdvertisedAddr`, and optional
`-MachineUuid` for dual-boot or multi-OS physical-machine grouping.

```powershell
.\packaging\windows\install-service.ps1 -BinaryPath .\target\release\telemon-exporter.exe -AddFirewallRule
```

Check:

```powershell
Get-Service TelemonExporter
Invoke-WebRequest http://127.0.0.1:9185/healthz
Invoke-WebRequest http://127.0.0.1:9185/metrics
```

Uninstall:

```powershell
.\packaging\windows\uninstall-service.ps1
```

The uninstall script preserves `C:\ProgramData\Telemon\exporter.yml`.
