param([Parameter(Mandatory = $true)][string]$TestExecutable)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Native loader diagnostics require Windows.' }
$executable = Get-Item -LiteralPath $TestExecutable
Write-Output "Native test executable: $($executable.FullName)"
Write-Output "Executable SHA-256: $((Get-FileHash -LiteralPath $executable.FullName -Algorithm SHA256).Hash)"

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$dumpbin = & $vswhere -latest -products '*' -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe' | Select-Object -First 1
if (-not $dumpbin) { throw 'Visual Studio dumpbin was not found.' }
Write-Output "Import inspector: $dumpbin"
Write-Output 'Native test dependencies:'
& $dumpbin /dependents $executable.FullName
Write-Output 'Native test imports:'
$imports = & $dumpbin /imports $executable.FullName
$imports | Write-Output
if ($LASTEXITCODE -ne 0) { throw 'Native test import inspection failed.' }

$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$mt = Get-ChildItem -Path "$kits/*/x64/mt.exe" | Sort-Object FullName -Descending | Select-Object -First 1
if ($mt) {
    $manifest = Join-Path $env:RUNNER_TEMP 'lomi-native-test.manifest.xml'
    Write-Output 'Embedded test executable manifest (resource 1):'
    & $mt.FullName -nologo "-inputresource:$($executable.FullName);#1" "-out:$manifest"
    Write-Output "Manifest inspector exit code: $LASTEXITCODE"
    if (Test-Path -LiteralPath $manifest) { Get-Content -LiteralPath $manifest }
} else {
    Write-Output 'Windows SDK manifest inspector was not found.'
}

$systemControls = Join-Path $env:SystemRoot 'System32/comctl32.dll'
$controlFile = Get-Item -LiteralPath $systemControls
Write-Output "System common controls DLL: $($controlFile.FullName)"
Write-Output "System common controls version: $($controlFile.VersionInfo.FileVersion)"
$exports = & $dumpbin /exports $systemControls
if ($LASTEXITCODE -ne 0) { throw 'System common controls export inspection failed.' }
$inControls = $false
foreach ($line in $imports) {
    if ($line -match '^\s+([^\s]+\.dll)\s*$') {
        $inControls = $Matches[1] -ieq 'COMCTL32.dll'
    } elseif ($inControls -and $line -match '^\s+[0-9A-F]+\s+([A-Za-z_][A-Za-z_0-9@?]*)\s*$') {
        $name = $Matches[1]
        $present = [bool]($exports | Where-Object { $_ -match "\s$([regex]::Escape($name))($|\s|=)" })
        Write-Output "Imported common controls entry point $name present in System32 exports: $present"
    }
}
Write-Output 'These are import/manifest receipts; System32 exports do not establish the failed process activation context.'
