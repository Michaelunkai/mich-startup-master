$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$iconPath = Join-Path $PSScriptRoot 'assets\MichStartupMaster.ico'
$previewPath = Join-Path $PSScriptRoot 'assets\MichStartupMaster.preview.png'
$sizes = @(16, 20, 24, 32, 40, 48, 64, 256)
$frames = New-Object System.Collections.Generic.List[byte[]]

function New-Frame([int]$size) {
    $bitmap = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.Clear([System.Drawing.Color]::Transparent)

        $margin = [Math]::Max(1, [int]($size * 0.06))
        $rect = New-Object System.Drawing.Rectangle $margin, $margin, ($size - ($margin * 2)), ($size - ($margin * 2))
        $background = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 12, 91, 93))
        $outline = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 83, 235, 202)), ([Math]::Max(1, $size / 20))
        $graphics.FillEllipse($background, $rect)
        $graphics.DrawEllipse($outline, $rect)

        $center = $size / 2
        $arrowPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White, ([Math]::Max(1, $size / 10)))
        $arrowPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
        $arrowPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
        $top = [int]($size * 0.24)
        $bottom = [int]($size * 0.72)
        $left = [int]($size * 0.31)
        $right = [int]($size * 0.69)
        $graphics.DrawLine($arrowPen, $center, $bottom, $center, $top)
        $graphics.DrawLine($arrowPen, $center, $top, $left, [int]($size * 0.43))
        $graphics.DrawLine($arrowPen, $center, $top, $right, [int]($size * 0.43))

        $stream = New-Object System.IO.MemoryStream
        try {
            $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
            return $stream.ToArray()
        } finally {
            $stream.Dispose()
        }
    } finally {
        if ($arrowPen) { $arrowPen.Dispose() }
        if ($outline) { $outline.Dispose() }
        if ($background) { $background.Dispose() }
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

foreach ($size in $sizes) {
    $frames.Add((New-Frame $size))
}

$stream = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $stream
try {
    $writer.Write([UInt16]0)
    $writer.Write([UInt16]1)
    $writer.Write([UInt16]$sizes.Count)
    $offset = 6 + (16 * $sizes.Count)
    for ($index = 0; $index -lt $sizes.Count; $index++) {
        $size = $sizes[$index]
        $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
        $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
        $writer.Write([byte]0)
        $writer.Write([byte]0)
        $writer.Write([UInt16]1)
        $writer.Write([UInt16]32)
        $writer.Write([UInt32]$frames[$index].Length)
        $writer.Write([UInt32]$offset)
        $offset += $frames[$index].Length
    }
    foreach ($frame in $frames) {
        $writer.Write($frame)
    }
    [System.IO.File]::WriteAllBytes($iconPath, $stream.ToArray())
} finally {
    $writer.Dispose()
    $stream.Dispose()
}

[System.IO.File]::WriteAllBytes($previewPath, $frames[$frames.Count - 1])
$icon = New-Object System.Drawing.Icon $iconPath
try {
    [pscustomobject]@{
        Icon = $iconPath
        Frames = $sizes -join ','
        ValidationSize = "$($icon.Width)x$($icon.Height)"
        Preview = $previewPath
    } | Format-List
} finally {
    $icon.Dispose()
}
