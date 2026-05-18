param(
    [string]$ServiceName = "TelemonExporter"
)

$ErrorActionPreference = "Stop"

Get-Service -Name $ServiceName
Invoke-WebRequest http://127.0.0.1:9185/healthz -UseBasicParsing
Invoke-WebRequest http://127.0.0.1:9185/metrics -UseBasicParsing
