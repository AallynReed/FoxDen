# Downloads VB-Audio's VB-CABLE pack, checks it and copies the x64 Windows 10 files into packaging/vbcable.
$ErrorActionPreference = 'Stop'
$url = 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip'
$sha = 'BC44E8974191A5A2427D6C979E2DAB08DBB09925F3F435DFA2F0BA3E45B6EB90'
$files = @(
    'VBCABLE_Setup_x64.exe',
    'vbMmeCable64_win10.inf',
    'vbaudio_cable64_win10.cat',
    'vbaudio_cable64_win10.sys',
    'vbaudio_cable64arm_win10.sys',
    'readme.txt'
)

$out = Join-Path $PSScriptRoot 'vbcable'
$work = Join-Path ([IO.Path]::GetTempPath()) ('vbcable-' + [guid]::NewGuid())
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $zip = Join-Path $work 'pack.zip'
    Invoke-WebRequest $url -OutFile $zip -UseBasicParsing
    if ((Get-FileHash $zip -Algorithm SHA256).Hash -ne $sha) { throw 'VB-CABLE pack hash mismatch' }
    Expand-Archive $zip (Join-Path $work 'pack')
    New-Item -ItemType Directory -Force $out | Out-Null
    foreach ($f in $files) { Copy-Item (Join-Path $work "pack\$f") (Join-Path $out $f) -Force }
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
