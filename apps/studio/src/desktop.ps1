$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase, System.Drawing, System.Windows.Forms
    Add-Type -Path (Join-Path $PSScriptRoot 'desktop.cs') -ReferencedAssemblies UIAutomationClient, UIAutomationTypes, WindowsBase, System.Drawing, System.Windows.Forms
    [KlyneDesktop]::CheckEmergency()
    $action = $request.action
    if ($action.tool -in @('desktop_click','desktop_scroll')) {
        $prior = $request.previous.windows | Where-Object { $_.window -eq $action.window } | Select-Object -First 1
        [KlyneDesktop]::CheckLayout([string]$action.window,[int]$prior.left,[int]$prior.top,[int]$prior.width,[int]$prior.height)
    }
    switch ($action.tool) {
        'desktop_observe' { }
        'desktop_apps' {
            $apps = @(Get-StartApps | Select-Object -First 400 Name,AppID)
            @{ok=$true; apps=$apps} | ConvertTo-Json -Depth 8 -Compress
            exit 0
        }
        'desktop_launch' {
            [KlyneDesktop]::InputStarted = $true
            $appId = [string]$action.app_id
            if ($appId -in @('notepad.exe','calc.exe')) { Start-Process -FilePath (Join-Path $env:WINDIR "System32\$appId") }
            else {
                $installed = Get-StartApps | Where-Object { $_.AppID -ceq $appId } | Select-Object -First 1
                if (!$installed) { throw 'App ID is not installed. Use desktop_apps first.' }
                Start-Process -FilePath (Join-Path $env:WINDIR 'explorer.exe') -ArgumentList @("shell:AppsFolder\$appId")
            }
            Start-Sleep -Milliseconds 600
        }
        'desktop_focus' { [KlyneDesktop]::Focus([string]$action.window) }
        'desktop_click' {
            $screen = $request.previous.screen
            $x = [int]$screen.left + [int]([double]$action.x * [double]$screen.width / [double]$screen.image_width)
            $y = [int]$screen.top + [int]([double]$action.y * [double]$screen.height / [double]$screen.image_height)
            [KlyneDesktop]::Click([string]$action.window,$x,$y,[string]$action.button)
        }
        'desktop_type' { [KlyneDesktop]::Type([string]$action.window,[string]$action.text) }
        'desktop_key' { [KlyneDesktop]::Press([string]$action.window,[string]$action.key) }
        'desktop_scroll' { [KlyneDesktop]::Scroll([string]$action.window,[int]$action.ticks) }
        'desktop_invoke' { [KlyneDesktop]::ElementAction([string]$action.window,[string]$action.element,'',$false) }
        'desktop_fill' { [KlyneDesktop]::ElementAction([string]$action.window,[string]$action.element,[string]$action.text,$true) }
        default { throw 'Unknown desktop action' }
    }
    Start-Sleep -Milliseconds 150
    $observation = [KlyneDesktop]::Observe([string]$request.image_path)
    @{ok=$true; observation=$observation} | ConvertTo-Json -Depth 12 -Compress
} catch {
    $failure = $_.Exception.Message + ' [' + $_.FullyQualifiedErrorId + ']'
    try {
        if (![KlyneDesktop]::InputStarted) {
            $observation = [KlyneDesktop]::Observe([string]$request.image_path)
            @{ok=$false; known_not_applied=$true; error=$failure; observation=$observation} | ConvertTo-Json -Depth 12 -Compress
            exit 0
        }
    } catch { }
    @{ok=$false; error=$failure} | ConvertTo-Json -Compress
    exit 1
}
