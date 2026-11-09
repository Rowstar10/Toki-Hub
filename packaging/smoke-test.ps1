# Tries out a built Toki Hub.exe: its built-in catalog, UI and API, then its window.
#   pwsh packaging/smoke-test.ps1 -Exe "dist/Toki Hub.exe"
param([string]$Exe = "dist/Toki Hub.exe")
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
$data = Join-Path ([IO.Path]::GetTempPath()) "tokihub-smoke-$PID"

function Stop-Hub { Get-Process -Name "Toki Hub" -ErrorAction SilentlyContinue | Stop-Process -Force }
function Quote([string]$text) { "`"$text`"" }   # Start-Process does not quote arguments itself

# 1. The catalog built into the exe loads without problems.
$check = Start-Process -FilePath $Exe -ArgumentList "--data-dir", (Quote $data), "check" -Wait -PassThru
if ($check.ExitCode -ne 0) { throw "Toki Hub.exe check failed (exit code $($check.ExitCode))" }
Write-Host "Built-in catalog: OK"

# 2. The UI, catalog images and API are all served from inside the exe.
$port = 8777
$base = "http://127.0.0.1:$port"
Start-Process -FilePath $Exe -ArgumentList "--browser", "--no-open", "--port", "$port", "--data-dir", (Quote $data) | Out-Null
try {
    $page = $null
    foreach ($i in 1..60) {
        try { $page = Invoke-WebRequest -UseBasicParsing "$base/"; break } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $page) { throw "Toki Hub.exe did not start its server" }
    $token = [regex]::Match($page.Content, 'name="toki-token" content="([^"]+)"').Groups[1].Value
    if (-not $token) { throw "The page has no token" }
    foreach ($path in "/css/hub.css", "/js/hub.js", "/fonts/exo-2-latin-wght-italic.woff2", "/img/toki-emblem.svg",
                      "/catalog/hu-taong/icon.png", "/catalog/hu-taong/cover.jpg") {
        $response = Invoke-WebRequest -UseBasicParsing "$base$path"
        if ($response.StatusCode -ne 200) { throw "$path was not served" }
    }
    $state = Invoke-RestMethod -Method Post -Uri "$base/api/state" -Headers @{ "X-Toki-Token" = $token } `
        -ContentType "application/json" -Body '{"args": []}'
    if (-not $state.ok) { throw "The API failed: $($state.error)" }
    if (-not $state.result.frozen) { throw "The exe does not know it is an exe" }
    $titles = @($state.result.games | ForEach-Object { $_.title })
    if ($titles -notcontains "Hu Taong") { throw "Hu Taong is missing from the built-in catalog" }
    Write-Host "UI and API: OK (Toki Hub $($state.result.hub_version); games: $($titles -join ', '))"
} finally {
    Stop-Hub
}

# 3. The window opens, which shows pywebview and WebView2 made it into the exe.
$windowData = "$data-window"
$logFile = Join-Path $windowData "logs/hub.log"
Start-Process -FilePath $Exe -ArgumentList "--data-dir", (Quote $windowData) | Out-Null
$shown = $false
foreach ($i in 1..60) {
    Start-Sleep -Milliseconds 500
    if ((Test-Path $logFile) -and (Select-String -Path $logFile -Pattern "Window shown" -Quiet)) { $shown = $true; break }
}
Stop-Hub
if (Test-Path $logFile) { Get-Content $logFile }
if ($shown) {
    Write-Host "Window: OK"
} else {
    Write-Host "::warning::The Toki Hub window did not report opening on the build machine (see hub.log above)."
}
