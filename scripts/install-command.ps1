param([string]$InstallDir = (Split-Path (Get-Command cargo -ErrorAction Stop).Source -Parent))
$ErrorActionPreference = 'Stop'
$launcher = Join-Path $PSScriptRoot 'start-klyne.ps1'
$target = Join-Path $InstallDir 'klyne.cmd'
$content = '@echo off' + "`r`n" + 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + $launcher + '" %*' + "`r`n" + 'exit /b %errorlevel%' + "`r`n"
if (Test-Path -LiteralPath $target) {
    if ((Get-Content -Raw -LiteralPath $target) -ne $content) {
        throw "An existing command occupies $target. It has been left intact."
    }
} else {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    [System.IO.File]::WriteAllText($target, $content, [System.Text.Encoding]::Default)
}
Write-Host "Installed $target. Type klyne in PowerShell to open the app."
