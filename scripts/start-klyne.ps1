param([switch]$NoBrowser, [string]$WarmModel = $(if ($env:KLYNE_WARM_MODEL) { $env:KLYNE_WARM_MODEL } else { 'qwen3.5:9b' }), [switch]$NoWarmup)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$root = Join-Path $repo 'workspace\studio'
$url = 'http://127.0.0.1:4317'
$browser = $null
if (-not $NoBrowser) {
    $browser = @(
        "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
        "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
        "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
        "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $browser) { throw 'Install Microsoft Edge or Chrome to open the Klyne app window.' }
}

function ConvertTo-KlyneComparablePath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { return $null }
    if ($Path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        $Path = '\\' + $Path.Substring(8)
    } elseif ($Path.StartsWith('\\?\')) {
        $Path = $Path.Substring(4)
    }
    return [System.IO.Path]::GetFullPath($Path)
}

function Get-KlyneProcess {
    try {
        $health = Invoke-RestMethod "$url/api/runtime/ready" -TimeoutSec 2
        $process = Get-Process -Id $health.pid -ErrorAction Stop
        $expected = Join-Path $repo 'target\debug\klyne-studio.exe'
        $current = Join-Path $root 'runtime\current.json'
        if (Test-Path -LiteralPath $current) {
            $active = Get-Content -Raw -LiteralPath $current | ConvertFrom-Json
            if ($active.binary) { $expected = $active.binary }
        }
        $actualPath = ConvertTo-KlyneComparablePath $process.Path
        $expectedPath = ConvertTo-KlyneComparablePath $expected
        if (-not $actualPath -or $actualPath -ne $expectedPath) { throw 'Unexpected server' }
        # Identity and serving health are separate from idle readiness. A recovered
        # task can be busy while this is already the correct running server.
        if ($health.root) {
            $actualRoot = ConvertTo-KlyneComparablePath $health.root
            $expectedRoot = ConvertTo-KlyneComparablePath $root
            if ($actualRoot -ne $expectedRoot) { throw 'Server uses a different Klyne data directory' }
        }
        return $process
    } catch { return $null }
}

# Warm the local model (2026-10-24) so the first request does not pay the
# 10-60 s load. Best effort, in its own hidden process so it overlaps the
# build check below; keep_alive/num_ctx match what Klyne sends.
function Start-KlyneModelWarmup {
    if ($NoWarmup -or [string]::IsNullOrWhiteSpace($WarmModel)) { return }
    try {
        $null = Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2
    } catch {
        Write-Host 'Ollama is not running; skipping model warm-up.'
        return
    }
    $body = (@{ model = $WarmModel; prompt = ''; keep_alive = '30m'; options = @{ num_ctx = 16384 } } | ConvertTo-Json -Compress).Replace("'", "''")
    $command = "try { Invoke-RestMethod -Method Post -Uri 'http://127.0.0.1:11434/api/generate' -ContentType 'application/json' -Body '$body' -TimeoutSec 180 | Out-Null } catch {}"
    # -EncodedCommand keeps the JSON quotes intact across the process boundary.
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-WindowStyle', 'Hidden', '-EncodedCommand', $encoded) -WindowStyle Hidden | Out-Null
    Write-Host "Warming $WarmModel in Ollama..."
}
Start-KlyneModelWarmup

$running = Get-KlyneProcess
$alreadyRunning = [bool]$running
if (-not $running) {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 4317)
    try { $listener.Start() }
    catch { throw "Port 4317 is occupied by another service. Close that service and run klyne again." }
    finally { $listener.Stop() }

    $supervisor = Join-Path $repo 'target\debug\klyne-supervisor.exe'
    $studio = Join-Path $repo 'target\debug\klyne-studio.exe'
    # Cargo's incremental check is cheap when unchanged. Existence alone can
    # keep launching an old binary after source fixes or embedded UI updates.
    Write-Host 'Checking the Klyne build...'
    Push-Location $repo
    try {
        & cargo build --locked -p klyne-studio --bins
        if ($LASTEXITCODE -ne 0) { throw 'Klyne build failed.' }
    } finally { Pop-Location }
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    $log = Join-Path $root ('launcher-' + [guid]::NewGuid().ToString('N'))
    $launchArgs = @('--root', ('"' + $root + '"'), '--port', '4317')
    if ($browser) { $launchArgs += @('--app-browser', ('"' + $browser + '"')) }
    $child = Start-Process -FilePath $supervisor -ArgumentList $launchArgs -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput "$log.out.log" -RedirectStandardError "$log.err.log"
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $running = Get-KlyneProcess
        if ($running) { break }
        if ($child.HasExited) { throw "Klyne could not start. See $log.err.log" }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $running) { throw "Klyne is still starting. See $log.err.log and try klyne again." }
}
if ($browser -and $alreadyRunning) {
    Start-Process -FilePath $browser -ArgumentList @("--app=$url", ('--user-data-dir="' + (Join-Path $root 'app-browser') + '"'), '--no-first-run', '--no-default-browser-check', '--disable-background-mode') -WindowStyle Normal
}
if ($browser) {
    $browserName = [System.IO.Path]::GetFileName($browser)
    $profile = Join-Path $root 'app-browser'
    $windowDeadline = [DateTime]::UtcNow.AddSeconds(30)
    $appWindow = $null
    do {
        $browserProcesses = Get-CimInstance Win32_Process -Filter "Name = '$browserName'"
        foreach ($candidate in $browserProcesses) {
            if ($candidate.CommandLine -and $candidate.CommandLine.Contains($profile) -and $candidate.CommandLine.Contains("--app=$url")) {
                $windowProcess = Get-Process -Id $candidate.ProcessId -ErrorAction SilentlyContinue
                if ($windowProcess -and $windowProcess.MainWindowHandle -ne 0) {
                    $appWindow = $windowProcess
                    break
                }
            }
        }
        if ($appWindow) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $windowDeadline)
    if (-not $appWindow) {
        throw "Klyne's server started, but its app window did not appear. You can open $url in your browser."
    }
    Write-Host "Klyne window is open at $url"
} else {
    Write-Host "Klyne is running at $url"
}
