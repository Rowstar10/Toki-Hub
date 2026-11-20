# Tries out a built Toki Hub.exe: its built-in catalog, then its window.
#   pwsh smoke-test.ps1 -Exe "dist/Toki Hub.exe"
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

# 3. Uninstalling (Settings > Uninstall, here from the command line): a copy in a folder whose name
#    needs quoting deletes itself once it has closed, and keeps its data unless told to wipe it.
foreach ($wipe in @($false, $true)) {
    $folder = Join-Path ([IO.Path]::GetTempPath()) "Toki Hub (uninstall $wipe) & co"
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    $copy = Join-Path $folder "Toki Hub.exe"
    Copy-Item $Exe $copy
    $own = Join-Path $folder "its data"
    Start-Process -FilePath $copy -ArgumentList "--data-dir", (Quote $own), "check" -Wait | Out-Null
    $arguments = @("--data-dir", (Quote $own), "uninstall")
    if ($wipe) { $arguments += "--wipe" }
    $run = Start-Process -FilePath $copy -ArgumentList $arguments -Wait -PassThru
    if ($run.ExitCode -ne 0) { Get-Content (Join-Path $own "logs/hub.log"); throw "Uninstall (wipe: $wipe) failed" }
    foreach ($i in 1..60) {
        if (-not (Test-Path -LiteralPath $copy) -and ($wipe -ne (Test-Path -LiteralPath $own))) { break }
        Start-Sleep -Milliseconds 500
    }
    if (Test-Path -LiteralPath $copy) { throw "Uninstall (wipe: $wipe) left Toki Hub.exe behind" }
    if ($wipe -and (Test-Path -LiteralPath $own)) { throw "Uninstall with wipe left the data folder behind" }
    if (-not $wipe -and -not (Test-Path -LiteralPath (Join-Path $own "cache"))) { throw "Uninstall without wipe deleted the data" }
    Remove-Item -LiteralPath $folder -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Uninstall (wipe: $wipe): OK"
}
