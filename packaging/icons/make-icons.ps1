# Generates the app icon (app.ico) and the Microsoft Store / MSIX tile assets.
# Pure System.Drawing, no external tools. Re-run after changing the design:
#   powershell -ExecutionPolicy Bypass -File packaging\icons\make-icons.ps1
Add-Type -AssemblyName System.Drawing

$root   = Split-Path -Parent $MyInvocation.MyCommand.Path
$assets = Join-Path $root '..\msix\Assets' | ForEach-Object { [System.IO.Path]::GetFullPath($_) }
New-Item -ItemType Directory -Force -Path $assets | Out-Null

# Same palette as the app (src/theme.rs).
$bg     = [System.Drawing.Color]::FromArgb(255, 21, 23, 28)
$panel  = [System.Drawing.Color]::FromArgb(255, 32, 35, 42)
$accent = [System.Drawing.Color]::FromArgb(255, 90, 125, 230)
$amber  = [System.Drawing.Color]::FromArgb(255, 210, 160, 60)

function New-RoundedRect([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

# A right-pointing block arrow (request) inside the box (x, y, w, h); mirrored for the response.
function Add-Arrow($g, [single]$x, [single]$y, [single]$w, [single]$h, $color, [bool]$pointsRight) {
    $shaft = $h * 0.42
    $head  = $w * 0.42
    $top   = ($h - $shaft) / 2
    $bot   = ($h + $shaft) / 2
    $neck  = $w - $head
    $mid   = $h / 2
    $pts = @(
        (0, $top), ($neck, $top), ($neck, 0), ($w, $mid), ($neck, $h), ($neck, $bot), (0, $bot)
    )
    $poly = @()
    foreach ($pt in $pts) {
        $px = if ($pointsRight) { $x + $pt[0] } else { $x + $w - $pt[0] }
        $poly += New-Object System.Drawing.PointF([single]$px, [single]($y + $pt[1]))
    }
    $brush = New-Object System.Drawing.SolidBrush $color
    $g.FillPolygon($brush, [System.Drawing.PointF[]]$poly)
    $brush.Dispose()
}

# Square icon: dark rounded tile, amber arrow going out, blue arrow coming back.
function New-IconBitmap([int]$size, [bool]$rounded = $true, [bool]$flat = $false) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)

    $pad = if ($flat) { 0 } else { [single]($size * 0.04) }
    $side = $size - 2 * $pad
    $radius = if ($rounded) { $side * 0.22 } else { 0.01 }
    if (-not $flat) {
        $path = New-RoundedRect $pad $pad $side $side $radius
        $g.FillPath((New-Object System.Drawing.SolidBrush $panel), $path)
        $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 60, 65, 76)), ([single][Math]::Max(1, $size * 0.012))
        $g.DrawPath($pen, $path)
    }

    $aw = $size * 0.56
    $ah = $size * 0.20
    $ax = ($size - $aw) / 2
    Add-Arrow $g $ax ($size * 0.28) $aw $ah $amber $true
    Add-Arrow $g $ax ($size * 0.54) $aw $ah $accent $false

    $g.Dispose()
    return $bmp
}

function Save-Png($bmp, [string]$path) {
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

# --- MSIX / Store tiles ------------------------------------------------------
$tiles = @{
    'StoreLogo.png'         = 50
    'Square44x44Logo.png'   = 44
    'Square71x71Logo.png'   = 71
    'Square150x150Logo.png' = 150
    'Square310x310Logo.png' = 310
}
foreach ($name in $tiles.Keys) { Save-Png (New-IconBitmap $tiles[$name]) (Join-Path $assets $name) }

# Taskbar / Start icons at fixed target sizes, with and without the plate.
foreach ($s in 16, 24, 32, 48, 256) {
    Save-Png (New-IconBitmap $s) (Join-Path $assets "Square44x44Logo.targetsize-$s.png")
    Save-Png (New-IconBitmap $s $false $true) (Join-Path $assets "Square44x44Logo.targetsize-${s}_altform-unplated.png")
}

# Wide 310x150 tile: icon on the left, name on the right.
$wide = New-Object System.Drawing.Bitmap 310, 150, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($wide)
$g.SmoothingMode = 'AntiAlias'
$g.TextRenderingHint = 'ClearTypeGridFit'
$g.Clear($bg)
$icon = New-IconBitmap 110
$g.DrawImage($icon, 22, 20, 110, 110)
$icon.Dispose()
$font1 = New-Object System.Drawing.Font 'Segoe UI Semibold', 17, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
$font2 = New-Object System.Drawing.Font 'Segoe UI', 13, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
$g.DrawString('Metamug', $font1, [System.Drawing.Brushes]::White, 142, 44)
$g.DrawString('API Tester', $font1, [System.Drawing.Brushes]::White, 142, 66)
$g.DrawString('fast, native, local', $font2, (New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 150, 158, 175))), 143, 94)
$g.Dispose()
Save-Png $wide (Join-Path $assets 'Wide310x150Logo.png')

# --- app.ico (PNG-compressed frames, Vista+) --------------------------------
$sizes = 16, 24, 32, 48, 64, 128, 256
$frames = foreach ($s in $sizes) {
    $bmp = New-IconBitmap $s
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    ,$ms.ToArray()
}
$ico = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ico
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $dim = if ($s -ge 256) { 0 } else { $s }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$frames[$i].Length); $w.Write([uint32]$offset)
    $offset += $frames[$i].Length
}
foreach ($f in $frames) { $w.Write($f) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $root 'app.ico'), $ico.ToArray())

# A large PNG for the website / Store listing (1024 px).
Save-Png (New-IconBitmap 1024) (Join-Path $root 'app-1024.png')
Save-Png (New-IconBitmap 300) (Join-Path $root 'store-logo-300.png')

Write-Host "Icons written to $root and $assets"

