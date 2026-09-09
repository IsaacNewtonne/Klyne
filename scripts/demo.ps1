$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    $demoWorkspace = Join-Path 'workspace' ('demo-' + [guid]::NewGuid().ToString('N'))
    cargo run -p harness-cli -- --workspace $demoWorkspace --objective 'create file hello.txt with content hello autonomous harness'
    if ($LASTEXITCODE -ne 0) { throw 'Demo execution failed' }
    $actual = Get-Content -LiteralPath (Join-Path $demoWorkspace 'hello.txt') -Raw
    if ($actual -cne 'hello autonomous harness') { throw 'Independent verification failed' }
    cargo run -p harness-cli -- --workspace $demoWorkspace --resume
    if ($LASTEXITCODE -ne 0) { throw 'Resume failed' }
    cargo run -p harness-cli -- --workspace $demoWorkspace --events
    if ($LASTEXITCODE -ne 0) { throw 'Event inspection failed' }
} finally {
    Pop-Location
}
