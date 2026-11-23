# Renders icon.svg into the icon files. Needs the Tauri CLI.
#   powershell -ExecutionPolicy Bypass -File native/icons/make-icons.ps1
param([string]$OutDir = $PSScriptRoot)

$ErrorActionPreference = 'Stop'
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
$work = Join-Path $PSScriptRoot '..\..\target\icons'
New-Item -ItemType Directory -Force $work | Out-Null

cargo tauri icon (Join-Path $PSScriptRoot 'icon.svg') --output $work | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'cargo tauri icon failed' }

# A build may read these at any moment, so each file is swapped in whole.
foreach ($name in '32x32.png', 'icon.png', 'icon.ico') {
    $path = Join-Path $OutDir $name
    Copy-Item -LiteralPath (Join-Path $work $name) -Destination "$path.tmp"
    if (Test-Path $path) {
        [System.IO.File]::Replace("$path.tmp", $path, [NullString]::Value)
    } else {
        [System.IO.File]::Move("$path.tmp", $path)
    }
    "{0,-16} {1,7} bytes" -f $name, (Get-Item -LiteralPath $path).Length
}
Remove-Item -Recurse -Force $work
