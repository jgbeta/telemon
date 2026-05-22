param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [string]$InstallDir = "C:\Program Files\Telemon",
    [string]$ConfigDir = "C:\ProgramData\Telemon",
    [string]$ServiceName = "TelemonExporter",
    [string]$PrometheusServerIp,
    [string]$RegistryServer,
    [string]$EnrollmentToken,
    [string]$UserName,
    [string]$DeviceName = $env:COMPUTERNAME,
    [string]$MachineUuid = $env:TELEMON_MACHINE_UUID,
    [string]$AdvertisedAddr,
    [ValidateSet("LocalService", "LocalSystem")]
    [string]$ServiceAccount = "LocalService",
    [switch]$AddFirewallRule
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $PSCommandPath
$defaultConfigPath = Join-Path $scriptDir "config.default.yml"

$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($currentIdentity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "install-service.ps1 must be run as Administrator"
}

if (-not (Test-Path $BinaryPath)) {
    throw "BinaryPath does not exist: $BinaryPath"
}

if ($PrometheusServerIp) {
    [System.Net.IPAddress]$parsedAddress = $null
    if (-not [System.Net.IPAddress]::TryParse($PrometheusServerIp, [ref]$parsedAddress)) {
        throw "PrometheusServerIp is not a valid IP address: $PrometheusServerIp"
    }
}

$userInteractive = [Environment]::UserInteractive
if (-not $RegistryServer -and $userInteractive) {
    $value = Read-Host "Registry server HOST:PORT (blank to disable registration)"
    $RegistryServer = $value
}
if ($RegistryServer) {
    if (-not $EnrollmentToken -and $userInteractive) {
        $secureToken = Read-Host "Enrollment token" -AsSecureString
        $EnrollmentToken = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
            [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
        )
    }
    if (-not $UserName -and $userInteractive) {
        $UserName = Read-Host "User name label"
    }
    if (-not $DeviceName) {
        $DeviceName = $env:COMPUTERNAME
    }
    if (-not $MachineUuid -and $userInteractive) {
        $MachineUuid = Read-Host "Machine UUID (blank for auto-generated local machine UUID)"
    }
    if (-not $EnrollmentToken -or -not $UserName) {
        Write-Warning "Registration disabled; registry server requires EnrollmentToken and UserName"
        $RegistryServer = ""
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $ConfigDir "state") | Out-Null

$targetBinary = Join-Path $InstallDir "telemon-exporter.exe"
$configPath = Join-Path $ConfigDir "exporter.yml"
$deviceIdPath = Join-Path $ConfigDir "state\device-id"
Copy-Item -Force -Path $BinaryPath -Destination $targetBinary

if (-not (Test-Path $configPath)) {
    if (Test-Path $defaultConfigPath) {
        Copy-Item -Path $defaultConfigPath -Destination $configPath
    } else {
@"
server:
  listen: "0.0.0.0:9185"
  metrics_path: "/metrics"
  static_metrics_path: "/metrics/static"

identity:
  user_name: ""
  device_name: ""
  machine_uuid: ""
  machine_uuid_file: ""

registration:
  enabled: false
  registry_addr: ""
  enrollment_token: ""
  device_id_file: ""
  heartbeat_interval_seconds: 30
  scrape_port: 9185
  advertised_addr: ""

collection:
  scrape_cache_stale_after_seconds: 60
  temperature_interval_seconds: 15
  sensor_rescan_interval_seconds: 300
  gpu_interval_seconds: 15
  windows_baseline_interval_seconds: 15
  windows_inventory_interval_seconds: 300
  static_info_interval_seconds: 300

adaptive_sampling:
  enabled: true
  levels:
    normal_seconds: 15
    warm_seconds: 10
    hot_seconds: 5
    critical_seconds: 1
  temperature:
    enabled: true
    warm_celsius: 60
    hot_celsius: 75
    critical_celsius: 85
  cooldown_seconds: 60

collectors:
  linux_hwmon:
    enabled: false
    root: "/sys/class/hwmon"
    include_unknown_sensors: false
    nvme_enrichment_enabled: true
    expose_storage_model: true
    sensor_allowlist: []
    sensor_denylist: []
  nvidia_nvml:
    enabled: true
    library_paths: []
    expose_gpu_name: true
    expose_gpu_uuid: false
    fan_speed_enabled: true
  windows_baseline:
    enabled: true
    include_removable_drives: false
    include_remote_drives: false
    network_interface_allowlist: []
    network_interface_denylist:
      - "loopback"
      - "isatap"
      - "teredo"
  windows_inventory:
    enabled: true

logging:
  level: "info"
"@ | Set-Content -Path $configPath -Encoding UTF8
    }
}

function Set-YamlScalar {
    param(
        [string]$Path,
        [string]$Section,
        [string]$Key,
        [string]$Value,
        [switch]$Raw
    )

    $currentSection = ""
    $lines = Get-Content -Path $Path
    $updated = foreach ($line in $lines) {
        if ($line -match '^([^ \t][^:]*):') {
            $currentSection = $Matches[1]
        }
        if ($currentSection -eq $Section -and $line -match "^\s+$([Regex]::Escape($Key)):\s*") {
            if ($Raw) {
                "  ${Key}: $Value"
            } else {
                "  ${Key}: ""$($Value.Replace('\', '\\').Replace('"', '\"'))"""
            }
        } else {
            $line
        }
    }
    $updated | Set-Content -Path $Path -Encoding UTF8
}

function Ensure-YamlSection {
    param(
        [string]$Path,
        [string]$Section,
        [string[]]$Lines
    )

    $content = Get-Content -Raw -Path $Path
    if ($content -notmatch "(?m)^$([Regex]::Escape($Section)):\s*$") {
        Add-Content -Path $Path -Encoding UTF8 -Value ""
        Add-Content -Path $Path -Encoding UTF8 -Value $Lines
    }
}

function Ensure-YamlScalarKey {
    param(
        [string]$Path,
        [string]$Section,
        [string]$Key,
        [string]$DefaultValue
    )

    $lines = @(Get-Content -Path $Path)
    $inSection = $false
    $found = $false
    $insertAt = $null

    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match '^([^ \t][^:]*):') {
            if ($inSection) {
                $insertAt = $i
                break
            }
            $inSection = ($Matches[1] -eq $Section)
            continue
        }
        if ($inSection -and $line -match "^\s+$([Regex]::Escape($Key)):\s*") {
            $found = $true
            break
        }
    }

    if ($found) {
        return
    }
    if ($inSection -and $null -eq $insertAt) {
        $insertAt = $lines.Count
    }
    if ($null -eq $insertAt) {
        return
    }

    $list = [System.Collections.Generic.List[string]]::new()
    $list.AddRange([string[]]$lines)
    $list.Insert($insertAt, "  ${Key}: $DefaultValue")
    $list | Set-Content -Path $Path -Encoding UTF8
}

Ensure-YamlSection -Path $configPath -Section "identity" -Lines @(
    "identity:",
    "  user_name: """"",
    "  device_name: """""
)
Ensure-YamlSection -Path $configPath -Section "registration" -Lines @(
    "registration:",
    "  enabled: false",
    "  registry_addr: """"",
    "  enrollment_token: """"",
    "  device_id_file: """"",
    "  heartbeat_interval_seconds: 30",
    "  scrape_port: 9185",
    "  advertised_addr: """""
)
Ensure-YamlScalarKey -Path $configPath -Section "identity" -Key "machine_uuid" -DefaultValue '""'
Ensure-YamlScalarKey -Path $configPath -Section "identity" -Key "machine_uuid_file" -DefaultValue '""'
Ensure-YamlScalarKey -Path $configPath -Section "registration" -Key "advertised_addr" -DefaultValue '""'
Ensure-YamlScalarKey -Path $configPath -Section "server" -Key "static_metrics_path" -DefaultValue '"/metrics/static"'
Ensure-YamlScalarKey -Path $configPath -Section "collection" -Key "windows_baseline_interval_seconds" -DefaultValue '15'
Ensure-YamlScalarKey -Path $configPath -Section "collection" -Key "windows_inventory_interval_seconds" -DefaultValue '300'
Ensure-YamlScalarKey -Path $configPath -Section "collection" -Key "static_info_interval_seconds" -DefaultValue '300'

if ($RegistryServer) {
    Set-YamlScalar -Path $configPath -Section "identity" -Key "user_name" -Value $UserName
    Set-YamlScalar -Path $configPath -Section "identity" -Key "device_name" -Value $DeviceName
    if ($MachineUuid) {
        Set-YamlScalar -Path $configPath -Section "identity" -Key "machine_uuid" -Value $MachineUuid
    }
    Set-YamlScalar -Path $configPath -Section "registration" -Key "enabled" -Value "true" -Raw
    Set-YamlScalar -Path $configPath -Section "registration" -Key "registry_addr" -Value $RegistryServer
    Set-YamlScalar -Path $configPath -Section "registration" -Key "enrollment_token" -Value $EnrollmentToken
    Set-YamlScalar -Path $configPath -Section "registration" -Key "device_id_file" -Value $deviceIdPath
    if ($AdvertisedAddr) {
        Set-YamlScalar -Path $configPath -Section "registration" -Key "advertised_addr" -Value $AdvertisedAddr
    }

    if (-not $PrometheusServerIp -and $RegistryServer.Contains(":")) {
        $PrometheusServerIp = $RegistryServer.Split(":")[0]
    }
}

$serviceAccountName = if ($ServiceAccount -eq "LocalSystem") { "LocalSystem" } else { "NT AUTHORITY\LocalService" }
if ($ServiceAccount -eq "LocalService") {
    icacls $ConfigDir /grant "NT AUTHORITY\LocalService:(OI)(CI)M" | Out-Null
}

$arguments = "service run --config `"$configPath`""
$service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($service) {
    if ($service.Status -ne "Stopped") {
        Stop-Service -Name $ServiceName -Force
    }
    sc.exe config $ServiceName binPath= "`"$targetBinary`" $arguments" start= auto obj= "$serviceAccountName" | Out-Null
} else {
    New-Service `
        -Name $ServiceName `
        -DisplayName "Telemon Exporter" `
        -BinaryPathName "`"$targetBinary`" $arguments" `
        -StartupType Automatic `
        -Description "Native Prometheus exporter for LAN hardware telemetry"
    sc.exe config $ServiceName obj= "$serviceAccountName" | Out-Null
}

if ($AddFirewallRule) {
    $ruleName = "Telemon Exporter 9185"
    if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Protocol TCP -LocalPort 9185 -Action Allow | Out-Null
    }
}

if ($PrometheusServerIp) {
    $ruleName = "Telemon Exporter 9185 from $PrometheusServerIp"
    if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule `
            -DisplayName $ruleName `
            -Direction Inbound `
            -Protocol TCP `
            -LocalPort 9185 `
            -RemoteAddress $PrometheusServerIp `
            -Profile Any `
            -Action Allow | Out-Null
    }
}

Start-Service -Name $ServiceName
Write-Host "Installed and started $ServiceName as $serviceAccountName"
