param([switch]$NoBrowser)
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
        return $process
    } catch { return $null }
}

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
