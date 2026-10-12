# Resize the complete dove artwork into Windows icon sizes, preserving its alpha.
# This script only writes electron/icon.ico inside this checkout.
Add-Type -AssemblyName System.Drawing
$artworkPath = Join-Path $PSScriptRoot '../public/columbina-icon.png'
$iconPath = Join-Path $PSScriptRoot 'icon.ico'
$artwork = [System.Drawing.Image]::FromFile((Resolve-Path -LiteralPath $artworkPath))
$sizes = @(16, 32, 48, 64, 128, 256)
$images = [System.Collections.Generic.List[byte[]]]::new()
try {
    $sourceRect = [System.Drawing.Rectangle]::new(0, 0, $artwork.Width, $artwork.Height)
    foreach ($size in $sizes) {
        $bitmap = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $stream = [System.IO.MemoryStream]::new()
        try {
            $graphics.Clear([System.Drawing.Color]::Transparent)
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $scale = [Math]::Min($size / $artwork.Width, $size / $artwork.Height)
            $width = [Math]::Max(1, [int][Math]::Round($artwork.Width * $scale))
            $height = [Math]::Max(1, [int][Math]::Round($artwork.Height * $scale))
            $targetRect = [System.Drawing.Rectangle]::new([int](($size - $width) / 2), [int](($size - $height) / 2), $width, $height)
            $graphics.DrawImage($artwork, $targetRect, $sourceRect, [System.Drawing.GraphicsUnit]::Pixel)
            $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
            $images.Add($stream.ToArray())
        } finally { $stream.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
    }
    $file = [System.IO.File]::Create($iconPath)
    $writer = [System.IO.BinaryWriter]::new($file)
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($index = 0; $index -lt $sizes.Count; $index++) {
            $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
            $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
            $writer.Write([byte]0); $writer.Write([byte]0)
            $writer.Write([uint16]1); $writer.Write([uint16]32)
            $writer.Write([uint32]$images[$index].Length); $writer.Write([uint32]$offset)
            $offset += $images[$index].Length
        }
        foreach ($bytes in $images) { $writer.Write($bytes) }
    } finally { $writer.Dispose(); $file.Dispose() }
} finally { $artwork.Dispose() }
Write-Output $iconPath
