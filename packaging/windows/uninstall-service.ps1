param(
    [string]$ServiceName = "TelemonExporter",
    [string]$InstallDir = "C:\Program Files\Telemon",
    [string]$ConfigDir = "C:\ProgramData\Telemon",
    [switch]$PreserveFirewall,
    [switch]$RemoveFiles
)

$ErrorActionPreference = "Stop"

$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($currentIdentity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "uninstall-service.ps1 must be run as Administrator"
}

$service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($service) {
    if ($service.Status -ne "Stopped") {
        Stop-Service -Name $ServiceName -Force
    }
    sc.exe delete $ServiceName | Out-Null
    Write-Host "Deleted $ServiceName"
} else {
    Write-Host "$ServiceName is not installed"
}

if ($PreserveFirewall) {
    Write-Host "Preserved Telemon firewall rules"
} else {
    $firewallRules = Get-NetFirewallRule -DisplayName "Telemon Exporter 9185*" -ErrorAction SilentlyContinue
    if ($firewallRules) {
        $firewallRules | Remove-NetFirewallRule
        Write-Host "Removed Telemon firewall rules"
    }
}

if ($RemoveFiles) {
    if (Test-Path $InstallDir) {
        Remove-Item -Recurse -Force $InstallDir
        Write-Host "Removed $InstallDir"
    }
    if (Test-Path $ConfigDir) {
        Remove-Item -Recurse -Force $ConfigDir
        Write-Host "Removed $ConfigDir"
    }
} else {
    Write-Host "Preserved $ConfigDir\exporter.yml"
}
