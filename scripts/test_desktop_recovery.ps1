param([switch]$Fixture,[string]$StateFile)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,WindowsBase,System.Drawing,System.Windows.Forms
if ($Fixture) {
    Add-Type -AssemblyName PresentationFramework
    $form=New-Object System.Windows.Window
    $form.Title='Klyne desktop recovery test'
    $form.Width=440; $form.Height=260
    $list=New-Object System.Windows.Controls.ListBox
    $null=$list.Items.Add('First choice');$null=$list.Items.Add('Second choice')
    $panel=New-Object System.Windows.Controls.StackPanel
    $list.Height=120;$null=$panel.Children.Add($list)
    $field=New-Object System.Windows.Controls.TextBox
    $field.Text='Before';$null=$panel.Children.Add($field)
    $notice=New-Object System.Windows.Controls.Button
    $notice.Content='Show test notice';$null=$panel.Children.Add($notice)
    $notice.Add_Click({
        $dialog=New-Object System.Windows.Window
        $dialog.Title='Klyne test notice';$dialog.Owner=$form;$dialog.Width=260;$dialog.Height=150
        $dismiss=New-Object System.Windows.Controls.Button;$dismiss.Content='Dismiss test notice'
        $dismiss.Add_Click({$dialog.Close()})
        $dialog.Content=$dismiss;$null=$dialog.ShowDialog()
    })
    $form.Content=$panel
    $form.Add_ContentRendered({ $handle=(New-Object System.Windows.Interop.WindowInteropHelper($form)).Handle.ToInt64().ToString(); @{window=$handle;list=$handle} | ConvertTo-Json | Set-Content -LiteralPath $StateFile })
    $list.Add_SelectionChanged({ [string]$list.SelectedItem | Set-Content -LiteralPath ($StateFile+'.selected') })
    $null=$form.ShowDialog()
    exit
}
Add-Type -Path (Join-Path $PSScriptRoot '../apps/studio/src/desktop.cs') -ReferencedAssemblies UIAutomationClient,UIAutomationTypes,WindowsBase,System.Drawing,System.Windows.Forms
$directory=Join-Path $PSScriptRoot '../workspace/desktop-recovery-test'
New-Item -ItemType Directory -Force $directory | Out-Null
$StateFile=Join-Path (Resolve-Path $directory).Path ([guid]::NewGuid().ToString()+'.json')
$process=Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList @('-STA','-NoProfile','-File',('"'+$PSCommandPath+'"'),'-Fixture','-StateFile',('"'+$StateFile+'"'))
try {
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    while (!(Test-Path -LiteralPath $StateFile)) { if([DateTime]::UtcNow -gt $deadline){throw 'Fixture startup timed out'};Start-Sleep -Milliseconds 100 }
    $state=Get-Content -LiteralPath $StateFile -Raw | ConvertFrom-Json
    $id=[string]$state.window
    Add-Type 'using System;using System.Runtime.InteropServices;public static class FixtureWindow{[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int command);}'
    $null=[FixtureWindow]::ShowWindow([IntPtr][long]$id,5)
    $null=[FixtureWindow]::ShowWindow([IntPtr][long]$id,9)
    Start-Sleep -Milliseconds 200
    [KlyneDesktop]::Focus($id)
    $null=[FixtureWindow]::ShowWindow([IntPtr][long]$id,6)
    Start-Sleep -Milliseconds 100
    $minimized=[KlyneDesktop]::Observe((Join-Path $directory 'minimized-test.png'))
    $found=$minimized.windows | Where-Object {$_.window -eq $id}
    if(!$found -or !$found.minimized){throw 'Minimized app missing from window discovery'}
    [KlyneDesktop]::Focus($id)
    $restored=[KlyneDesktop]::Observe((Join-Path $directory 'restored-test.png'))
    if($restored.foreground -ne $id -or ($restored.windows | Where-Object {$_.window -eq $id}).minimized){throw 'Minimized app was not restored and focused'}
    $identity=$restored.windows | Where-Object {$_.window -eq $id}
    if($identity.process_id -ne $process.Id -or !$identity.process_started){throw 'Original process identity was not observed'}
    if($identity.process_started -ne $found.process_started){throw 'Process identity changed across window restoration'}
    'PASS: process lifetime identity matches the fixture and survives restoration.'
    $field=$restored.controls | Where-Object {$_.type -eq 'ControlType.Edit'} | Select-Object -First 1
    if(!$field){throw 'Fixture field missing'}
    [KlyneDesktop]::ElementAction($id,[string]$field.element,'Reconciled field',$true)
    $filled=[KlyneDesktop]::Observe((Join-Path $directory 'filled-test.png'))
    $value=$filled.controls | Where-Object {$_.element -eq $field.element} | Select-Object -First 1
    if($value.value -ne 'Reconciled field' -or $value.value_truncated){throw 'Field fill evidence does not match'}
    [KlyneDesktop]::ElementAction($id,[string]$field.element,('x'*1300),$true)
    $long=[KlyneDesktop]::Observe((Join-Path $directory 'long-field-test.png'))
    $value=$long.controls | Where-Object {$_.element -eq $field.element} | Select-Object -First 1
    if(!$value.value_truncated){throw 'Truncated field evidence was not marked'}
    'PASS: exact field value observed; truncated values explicitly marked.'
    [KlyneDesktop]::InputStarted=$false
    $rejected=$false
    try {[KlyneDesktop]::CheckLayout($id,-999,-999,1,1)}catch{$rejected=$true}
    if(!$rejected -or [KlyneDesktop]::InputStarted){throw 'Stale layout sent input'}
    $element=[System.Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]$state.list)
    $items=$element.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
    $target=$items | Where-Object {$_.Current.Name -eq 'Second choice'} | Select-Object -First 1
    if(!$target){throw ('Accessibility test item missing: root='+$element.Current.Name+' type='+$element.Current.ControlType.ProgrammaticName+' children='+(($items | ForEach-Object {$_.Current.Name+':'+$_.Current.ControlType.ProgrammaticName}) -join ','))}
    [KlyneDesktop]::ElementAction($id,[string]::Join('.',$target.GetRuntimeId()),'',$false)
    Start-Sleep -Milliseconds 200
    if((Get-Content -LiteralPath ($StateFile+'.selected')).Trim() -ne 'Second choice'){throw 'Selection fallback did not change app state'}
    $root=[System.Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]$id)
    $notice=$root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,(New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty,'Show test notice')))
    [KlyneDesktop]::ElementAction($id,[string]::Join('.',$notice.GetRuntimeId()),'',$false)
    Start-Sleep -Milliseconds 200
    $image=Join-Path $directory 'test-observation.png'
    $observation=[KlyneDesktop]::Observe($image)
    $popup=$observation.windows | Where-Object {$_.owner -eq $id -and $_.title -eq 'Klyne test notice'} | Select-Object -First 1
    if(!$popup){throw 'Owned popup not identified'}
    $parent=$observation.windows | Where-Object {$_.window -eq $id}
    if($parent.enabled){throw 'Modal parent should be disabled'}
    $dismiss=$observation.controls | Where-Object {$_.name -eq 'Dismiss test notice' -and $_.type -eq 'ControlType.Button'} | Select-Object -First 1
    [KlyneDesktop]::ElementAction([string]$popup.window,[string]$dismiss.element,'',$false)
    'PASS: stale layout sends no input; selection changes app state; owned modal detected and dismissed.' 
} finally {
    if(!$process.HasExited){$null=$process.CloseMainWindow();if(!$process.WaitForExit(2000)){$process.Kill()}}
}
