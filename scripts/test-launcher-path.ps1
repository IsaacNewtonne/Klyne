$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot 'start-klyne.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Launcher parse failed' }
$function = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'ConvertTo-KlyneComparablePath' }, $true)
. ([scriptblock]::Create($function.Extent.Text))
foreach ($pair in @(
    @('C:\Klyne\studio.exe', '\\?\C:\Klyne\studio.exe'),
    @('C:\Klyne\studio.exe', '\\?\c:\klyne\studio.exe'),
    @('\\server\share\studio.exe', '\\?\UNC\server\share\studio.exe')
)) {
    if ((ConvertTo-KlyneComparablePath $pair[0]) -ne (ConvertTo-KlyneComparablePath $pair[1])) { throw 'Equivalent executable paths did not match' }
}
if ((ConvertTo-KlyneComparablePath 'C:\Klyne\studio.exe') -eq (ConvertTo-KlyneComparablePath 'C:\Other\studio.exe')) { throw 'Different executables matched' }
if ($null -ne (ConvertTo-KlyneComparablePath '')) { throw 'Empty executable path accepted' }
Write-Host 'Launcher executable path checks passed.'
