# Klyne Chrome launcher (2026-10-24, opt-in "Allow using my real Chrome").
# Starts YOUR Chrome profile with remote debugging on 127.0.0.1:9222 so Klyne's
# browser_attach can control your real tabs. Chrome only accepts the flag when
# no other Chrome window is running, so close Chrome first.
# Klyne never reads Chrome cookie or password files.
param([int]$Port = 9222)
$ErrorActionPreference = 'Stop'
$chrome = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $chrome) { throw 'Google Chrome was not found.' }
if (Get-Process chrome -ErrorAction SilentlyContinue) {
    throw 'Chrome is already running. Close every Chrome window (and the tray icon), then run this again.'
}
Start-Process -FilePath $chrome -ArgumentList @("--remote-debugging-port=$Port", '--remote-debugging-address=127.0.0.1')
$deadline = [DateTime]::UtcNow.AddSeconds(15)
do {
    try {
        $version = Invoke-RestMethod "http://127.0.0.1:$Port/json/version" -TimeoutSec 2
        Write-Host "Chrome is ready for Klyne at http://127.0.0.1:$Port ($($version.Browser))."
        Write-Host "In Klyne Settings, turn on 'Allow using my real Chrome'."
        return
    } catch { Start-Sleep -Milliseconds 500 }
} while ([DateTime]::UtcNow -lt $deadline)
Write-Warning "Chrome started but the debugging port did not answer. Recent Chrome versions refuse remote debugging on the default profile; in that case Klyne falls back to its own isolated browser."
