# Generates the app icon (app.ico), the in-app window icon (window-128.rgba) and a large PNG.
# The artwork is the same plunger as docs/images/plunger.svg.
# Pure System.Drawing, no external tools. Re-run after changing the design:
#   powershell -ExecutionPolicy Bypass -File packaging\icons\make-icons.ps1
Add-Type -AssemblyName System.Drawing

$root   = Split-Path -Parent $MyInvocation.MyCommand.Path

# Same palette as the app (src/theme.rs).
$bg     = [System.Drawing.Color]::FromArgb(255, 21, 23, 28)
$panel  = [System.Drawing.Color]::FromArgb(255, 32, 35, 42)
$muted  = [System.Drawing.Color]::FromArgb(255, 150, 158, 175)

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

# The plunger, drawn in a 100-unit box scaled to $size: wooden handle, red rubber cup.
# Fine detail (highlight) is skipped at small sizes; the handle never gets thinner than 2.4 px.
function Add-Plunger($g, [single]$size) {
    $u = $size / 100.0
    $woodA = [System.Drawing.Color]::FromArgb(255, 218, 166, 98)
    $woodB = [System.Drawing.Color]::FromArgb(255, 162, 106, 54)
    $redA  = [System.Drawing.Color]::FromArgb(255, 234, 92, 80)
    $redB  = [System.Drawing.Color]::FromArgb(255, 160, 38, 30)
    $lipA  = [System.Drawing.Color]::FromArgb(255, 186, 50, 42)
    $lipB  = [System.Drawing.Color]::FromArgb(255, 132, 26, 20)
    $horiz = [System.Drawing.Drawing2D.LinearGradientMode]::Horizontal

    # handle
    $hw = [Math]::Max(8 * $u, 2.4)
    $hx = 50 * $u - $hw / 2
    $handle = New-RoundedRect $hx (11 * $u) $hw (56 * $u) ($hw / 2)
    $rect = New-Object System.Drawing.RectangleF ([single]$hx), ([single](11 * $u)), ([single]$hw), ([single](56 * $u))
    $g.FillPath((New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, $woodA, $woodB, $horiz), $handle)

    # neck
    $neck = [System.Drawing.PointF[]]@(
        (New-Object System.Drawing.PointF ([single](43 * $u)), ([single](62 * $u))),
        (New-Object System.Drawing.PointF ([single](57 * $u)), ([single](62 * $u))),
        (New-Object System.Drawing.PointF ([single](59 * $u)), ([single](71 * $u))),
        (New-Object System.Drawing.PointF ([single](41 * $u)), ([single](71 * $u))))
    $g.FillPolygon((New-Object System.Drawing.SolidBrush $lipB), $neck)

    # cup: a dome from (22, 88) over (50, 64) to (78, 88)
    $cup = New-Object System.Drawing.Drawing2D.GraphicsPath
    $cup.AddBezier([single](22 * $u), [single](88 * $u), [single](22 * $u), [single](74 * $u), [single](34 * $u), [single](64 * $u), [single](50 * $u), [single](64 * $u))
    $cup.AddBezier([single](50 * $u), [single](64 * $u), [single](66 * $u), [single](64 * $u), [single](78 * $u), [single](74 * $u), [single](78 * $u), [single](88 * $u))
    $cup.CloseFigure()
    $crect = New-Object System.Drawing.RectangleF ([single](22 * $u)), ([single](64 * $u)), ([single](56 * $u)), ([single](24 * $u))
    $g.FillPath((New-Object System.Drawing.Drawing2D.LinearGradientBrush $crect, $redA, $redB, $horiz), $cup)

    # rim
    $lrect = New-Object System.Drawing.RectangleF ([single](22 * $u)), ([single](84.4 * $u)), ([single](56 * $u)), ([single](7.2 * $u))
    $g.FillEllipse((New-Object System.Drawing.Drawing2D.LinearGradientBrush $lrect, $lipA, $lipB, $horiz), $lrect)

    # soft highlight on the cup
    if ($size -ge 48) {
        $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(80, 255, 255, 255)), ([single][Math]::Max(1.2, 2.6 * $u))
        $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
        $g.DrawBezier($pen, [single](29 * $u), [single](83 * $u), [single](30 * $u), [single](75 * $u), [single](37 * $u), [single](69 * $u), [single](45 * $u), [single](67 * $u))
    }
}

# Square icon: dark rounded tile with the plunger on it. $flat drops the tile (unplated variants).
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

    Add-Plunger $g $size

    $g.Dispose()
    return $bmp
}

function Save-Png($bmp, [string]$path) {
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

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

# A large PNG of the icon (1024 px).
Save-Png (New-IconBitmap 1024) (Join-Path $root 'app-1024.png')

# Window / taskbar icon used at runtime: 128x128 raw RGBA (unmultiplied), loaded by src/main.rs.
$win = New-IconBitmap 128
$rgba = New-Object byte[] (128 * 128 * 4)
for ($y = 0; $y -lt 128; $y++) {
    for ($x = 0; $x -lt 128; $x++) {
        $c = $win.GetPixel($x, $y); $i = ($y * 128 + $x) * 4
        $rgba[$i] = $c.R; $rgba[$i + 1] = $c.G; $rgba[$i + 2] = $c.B; $rgba[$i + 3] = $c.A
    }
}
$win.Dispose()
[System.IO.File]::WriteAllBytes((Join-Path $root 'window-128.rgba'), $rgba)

Write-Host "Icons written to $root and $assets"

