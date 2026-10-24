$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase, System.Drawing, System.Windows.Forms
    Add-Type -Path (Join-Path $PSScriptRoot 'desktop.cs') -ReferencedAssemblies UIAutomationClient, UIAutomationTypes, WindowsBase, System.Drawing, System.Windows.Forms
    [KlyneDesktop]::CheckEmergency()
    $action = $request.action
    $launched = $null
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
            # 2026-10-24: names match loosely, and the new window comes back
            # as launched {window,pid} already focused, ready for typing.
            $appId = [string]$action.app_id
            # Only a process started by this launch counts: apps such as
            # Windows 11 Notepad may open a tab in the user's existing window,
            # and that window must never be handed back as "launched".
            $before = @{}
            $beforeWin = @{}
            foreach ($p in Get-Process) { $before[$p.Id] = $true; if ($p.MainWindowHandle -ne 0) { $beforeWin[[string]$p.MainWindowHandle.ToInt64()] = $true } }
            $builtin = @{ 'notepad'='notepad.exe'; 'notepad.exe'='notepad.exe'; 'calc'='calc.exe'; 'calc.exe'='calc.exe'; 'calculator'='calc.exe' }
            $exe = $builtin[$appId.ToLowerInvariant()]
            if ($exe) {
                [KlyneDesktop]::InputStarted = $true
                Start-Process -FilePath (Join-Path $env:WINDIR "System32\$exe")
                $want = @($exe -replace '\.exe$','')
                if ($exe -eq 'calc.exe') { $want += 'CalculatorApp' }
            } else {
                $apps = @(Get-StartApps)
                $installed = $apps | Where-Object { $_.AppID -ceq $appId } | Select-Object -First 1
                $bare = $appId -replace '\.exe$',''
                if (!$installed) {
                    $close = @($apps | Where-Object { $_.AppID -ieq $appId -or $_.Name -ieq $appId -or $_.Name -ieq $bare })
                    if ($close.Count -eq 1) { $installed = $close[0] }
                }
                if (!$installed) {
                    $hint = @($apps | Where-Object { $_.Name -like "*$bare*" -or $_.AppID -like "*$bare*" } | Select-Object -First 5 | ForEach-Object { "$($_.Name) => $($_.AppID)" })
                    if ($hint.Count) { throw ("App ID '$appId' is not installed. Did you mean: " + ($hint -join '; ') + '? Pass the AppID after =>.') }
                    throw "App ID '$appId' is not installed. Use desktop_apps to list installed IDs."
                }
                [KlyneDesktop]::InputStarted = $true
                Start-Process -FilePath (Join-Path $env:WINDIR 'explorer.exe') -ArgumentList @("shell:AppsFolder\$($installed.AppID)")
                $want = @()
            }
            for ($i = 0; $i -lt 25 -and !$launched; $i++) {
                Start-Sleep -Milliseconds 200
                $fresh = @(Get-Process | Where-Object { $_.MainWindowHandle -ne 0 -and !$before.ContainsKey($_.Id) })
                $pick = @($fresh | Where-Object { $want -contains $_.ProcessName }) + @($fresh) | Select-Object -First 1
                if (!$pick -and $want.Count) {
                    # Reused process (UWP/single-instance apps): accept only a
                    # window handle that did not exist before this launch.
                    $pick = @(Get-Process | Where-Object { $_.MainWindowHandle -ne 0 -and $want -contains $_.ProcessName -and !$beforeWin.ContainsKey([string]$_.MainWindowHandle.ToInt64()) }) | Select-Object -First 1
                }
                if ($pick) { $launched = @{ window = [string]$pick.MainWindowHandle.ToInt64(); pid = $pick.Id; process_name = $pick.ProcessName; title = $pick.MainWindowTitle; new_window = $true; reused_process = [bool]$before.ContainsKey($pick.Id) } }
            }
            if ($launched) { try { [KlyneDesktop]::Focus($launched.window) } catch { $launched['focus_error'] = $_.Exception.Message } }
            else { $launched = @{ window = $null; note = 'No new window appeared. The app may have opened inside an existing window (for example a new Notepad tab in the user''s window). Do not type into existing windows; stop and report this.' } }
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
        'desktop_drag' {
            $screen = $request.previous.screen
            $x1 = [int]$screen.left + [int]([double]$action.x1 * [double]$screen.width / [double]$screen.image_width)
            $y1 = [int]$screen.top + [int]([double]$action.y1 * [double]$screen.height / [double]$screen.image_height)
            $x2 = [int]$screen.left + [int]([double]$action.x2 * [double]$screen.width / [double]$screen.image_width)
            $y2 = [int]$screen.top + [int]([double]$action.y2 * [double]$screen.height / [double]$screen.image_height)
            [KlyneDesktop]::Drag([string]$action.window,$x1,$y1,$x2,$y2)
        }
        'desktop_clipboard_get' { $clipboard = [KlyneDesktop]::ClipboardGet() }
        'desktop_clipboard_set' { [KlyneDesktop]::ClipboardSet([string]$action.text) }
        default { throw 'Unknown desktop action' }
    }
    Start-Sleep -Milliseconds 150
    $observation = [KlyneDesktop]::Observe([string]$request.image_path)
    $result = @{ok=$true; observation=$observation}
    if ($action.tool -eq 'desktop_clipboard_get') { $result['clipboard'] = $clipboard }
    if ($launched) { $result['launched'] = $launched }
    $result | ConvertTo-Json -Depth 12 -Compress
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
