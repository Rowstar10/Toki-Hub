# Tries out a built Toki Hub.exe (Tauri version): its built-in catalog, then its window.
#   pwsh tauri/smoke-test.ps1 -Exe "tauri/dist/Toki Hub.exe"
param([string]$Exe = "dist/Toki Hub.exe")
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
$data = Join-Path ([IO.Path]::GetTempPath()) "tokihub-tauri-smoke-$PID"
$log = Join-Path $data "logs/hub.log"

function Quote([string]$text) { "`"$text`"" }   # Start-Process does not quote arguments itself
function Stop-Hub { Get-Process -Name "Toki Hub" -ErrorAction SilentlyContinue | Stop-Process -Force }

# 1. The catalog built into the exe loads without problems.
$check = Start-Process -FilePath $Exe -ArgumentList "--data-dir", (Quote $data), "check" -Wait -PassThru
if (Test-Path $log) { Get-Content $log }
if ($check.ExitCode -ne 0) { throw "Toki Hub.exe check failed (exit code $($check.ExitCode))" }
Write-Host "Built-in catalog: OK"

# 2. The window opens and its page talks to the Rust side ("UI loaded" is logged by the first call).
Start-Process -FilePath $Exe -ArgumentList "--data-dir", (Quote $data) | Out-Null
$loaded = $false
foreach ($i in 1..120) {
    Start-Sleep -Milliseconds 500
    if ((Test-Path $log) -and (Select-String -Path $log -Pattern "UI loaded" -Quiet)) { $loaded = $true; break }
}
Start-Sleep -Seconds 1
Stop-Hub
if (Test-Path $log) { Get-Content $log }
if (-not $loaded) { throw "The window did not load its page (see hub.log above)" }
Write-Host "Window and UI: OK"
