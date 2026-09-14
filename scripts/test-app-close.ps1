$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$root = Join-Path $repo ('workspace\app-close-test-' + [guid]::NewGuid().ToString('N'))
$browser = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$supervisor = Start-Process (Join-Path $repo 'target\debug\klyne-supervisor.exe') -ArgumentList @('--root', ('"' + $root + '"'), '--port', $port, '--app-browser', ('"' + $browser + '"')) -WindowStyle Hidden -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    $window = $null
    do {
        $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($supervisor.Id)"
        foreach ($item in $children) {
            $candidate = Get-Process -Id $item.ProcessId -ErrorAction SilentlyContinue
            if ($candidate -and $candidate.MainWindowHandle -ne 0) { $window = $candidate; break }
        }
        if ($window) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $window) { throw 'App window did not open.' }
    $health = Invoke-RestMethod "http://127.0.0.1:$port/api/runtime/ready" -TimeoutSec 3
    $owned = @($supervisor.Id)
    $all = @(Get-CimInstance Win32_Process)
    do {
        $added = @($all | Where-Object { $_.ParentProcessId -in $owned -and $_.ProcessId -notin $owned } | ForEach-Object { $_.ProcessId })
        $owned += $added
    } while ($added.Count -gt 0)
    if ($health.pid -notin $owned) { throw 'Studio is not in the owned process tree.' }
    $identities = @($owned | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue } | Select-Object Id,StartTime)
    if (-not $window.CloseMainWindow()) { throw 'Could not close the app window.' }
    if (-not $supervisor.WaitForExit(15000)) { throw 'Supervisor survived closing the app.' }
    Start-Sleep -Milliseconds 500
    foreach ($identity in $identities) {
        $alive = Get-Process -Id $identity.Id -ErrorAction SilentlyContinue
        if ($alive -and $alive.StartTime -eq $identity.StartTime) { throw "Owned process survived: $($identity.Id)" }
    }
    Write-Output "PASS: closing the app stopped all $($identities.Count) observed owned processes, including Studio and supervisor."
} finally {
    if (-not $supervisor.HasExited) { Stop-Process -InputObject $supervisor -Force }
}
