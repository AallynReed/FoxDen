# Draws the FoxRGB icon and writes every icon file Tauri reads.
#   powershell -ExecutionPolicy Bypass -File src-tauri/icons/make-icons.ps1
param([string]$OutDir = $PSScriptRoot)

$ErrorActionPreference = 'Stop'
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
Add-Type -AssemblyName System.Drawing

$orange = '#e8822e'
$shade  = '#c4661f'
$pale   = '#ead7bd'
$dim    = '#cbb597'
$dark   = '#17181a'

# An 8 x 8 grid keeps the outline on whole pixels at every size.
$outline = 0,0, 3,3, 5,3, 8,0, 8,4, 4,8, 0,4

# Painted over an orange square, which is then cut to the outline.
$facets = @(
    @($shade, (4,0, 8,0, 8,8, 4,8)),
    @($pale,  (0,4, 3,4, 3,6, 4,7, 4,8, 0,8)),
    @($dim,   (8,4, 5,4, 5,6, 4,7, 4,8, 8,8)),
    @($dark,  (3,6, 5,6, 4,7)),
    @($dark,  (2,4, 3,4, 3,5)),
    @($dark,  (6,4, 5,4, 5,5)),
    @($dark,  (0.5,1.5, 2,3, 0.5,3)),
    @($dark,  (7.5,1.5, 6,3, 7.5,3))
)

function Points($xy, $cell) {
    $points = New-Object System.Drawing.PointF[] ($xy.Count / 2)
    for ($i = 0; $i -lt $points.Count; $i++) {
        $points[$i] = New-Object System.Drawing.PointF ($xy[2 * $i] * $cell), ($xy[2 * $i + 1] * $cell)
    }
    , $points
}

function Canvas($size) {
    $bitmap = New-Object System.Drawing.Bitmap $size, $size
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = 'AntiAlias'
    $graphics.PixelOffsetMode = 'Half'
    $bitmap, $graphics
}

# Windows darkens half-transparent edge pixels when it shrinks the icon, so the outline has none.
function Harden($bitmap) {
    $area = New-Object System.Drawing.Rectangle 0, 0, $bitmap.Width, $bitmap.Height
    $data = $bitmap.LockBits($area, 'ReadWrite', 'Format32bppArgb')
    $bytes = New-Object byte[] ($data.Stride * $data.Height)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
    for ($i = 3; $i -lt $bytes.Length; $i += 4) {
        $bytes[$i] = if ($bytes[$i] -ge 96) { 255 } else { 0 }
    }
    [System.Runtime.InteropServices.Marshal]::Copy($bytes, 0, $data.Scan0, $bytes.Length)
    $bitmap.UnlockBits($data)
}

function Png($size) {
    $cell = $size / 8
    $face, $g = Canvas $size
    $g.Clear([System.Drawing.ColorTranslator]::FromHtml($orange))
    foreach ($facet in $facets) {
        $brush = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($facet[0]))
        $g.FillPolygon($brush, (Points $facet[1] $cell))
        $brush.Dispose()
    }
    $g.Dispose()

    $icon, $g = Canvas $size
    $texture = New-Object System.Drawing.TextureBrush $face
    $g.FillPolygon($texture, (Points $outline $cell))
    $texture.Dispose()
    $g.Dispose()
    $face.Dispose()
    Harden $icon

    $stream = New-Object System.IO.MemoryStream
    $icon.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $icon.Dispose()
    , $stream.ToArray()
}

# PNG entries, smallest first, as `tauri icon` writes them.
function Ico($sizes) {
    $images = @($sizes | ForEach-Object { , (Png $_) })
    $stream = New-Object System.IO.MemoryStream
    $writer = New-Object System.IO.BinaryWriter $stream
    $writer.Write([uint16]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $side = [byte]($sizes[$i] % 256)
        $writer.Write($side)
        $writer.Write($side)
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]32)
        $writer.Write([uint32]$images[$i].Length)
        $writer.Write([uint32]$offset)
        $offset += $images[$i].Length
    }
    foreach ($image in $images) { $writer.Write([byte[]]$image) }
    , $stream.ToArray()
}

# A build may read these at any moment, so each file is swapped in whole.
function Save($name, [byte[]]$bytes) {
    $path = Join-Path $OutDir $name
    [System.IO.File]::WriteAllBytes("$path.tmp", $bytes)
    if (Test-Path $path) {
        [System.IO.File]::Replace("$path.tmp", $path, [NullString]::Value)
    } else {
        [System.IO.File]::Move("$path.tmp", $path)
    }
    "{0,-16} {1,7} bytes" -f $name, $bytes.Length
}

Save '32x32.png' (Png 32)
Save '128x128.png' (Png 128)
Save '128x128@2x.png' (Png 256)
Save 'icon.png' (Png 512)
Save 'icon.ico' (Ico 16, 24, 32, 48, 64, 256)
