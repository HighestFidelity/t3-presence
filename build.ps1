# Builds T3Presence.exe next to this script: the multi-size icon with System.Drawing, then cargo (Rust, MSVC toolchain).
$ErrorActionPreference = 'Stop'
$dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$src = Join-Path $dir 'src'
$gen = Join-Path $dir 'build'
New-Item -ItemType Directory -Force $gen | Out-Null
Add-Type -AssemblyName System.Drawing

function Resized([System.Drawing.Image]$img, [int]$s) {
  $b = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($b)
  $g.InterpolationMode = 'HighQualityBicubic'; $g.SmoothingMode = 'HighQuality'; $g.PixelOffsetMode = 'HighQuality'
  $g.DrawImage($img, 0, 0, $s, $s); $g.Dispose()
  return $b
}

$logo = [System.Drawing.Image]::FromFile((Join-Path $dir 't3-logo-1024.png'))

# Multi-size .ico. 256 is stored as PNG, the rest as 32-bit DIB so System.Drawing.Icon can load them for the tray.
$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$frames = foreach ($s in $sizes) {
  $b = Resized $logo $s
  if ($s -eq 256) {
    $ms = New-Object System.IO.MemoryStream
    $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    , $ms.ToArray()
  } else {
    $ms = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $ms
    $mask = [int]([math]::Ceiling($s / 32.0)) * 4 * $s
    $w.Write([int]40); $w.Write([int]$s); $w.Write([int]($s * 2)); $w.Write([int16]1); $w.Write([int16]32); $w.Write([int]0); $w.Write([int]($s * $s * 4 + $mask)); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0)
    for ($y = $s - 1; $y -ge 0; $y--) { for ($x = 0; $x -lt $s; $x++) { $w.Write([int]$b.GetPixel($x, $y).ToArgb()) } }
    $w.Write((New-Object byte[] $mask))
    , $ms.ToArray()
  }
  $b.Dispose()
}
$logo.Dispose()
$ico = Join-Path $gen 'T3Presence.ico'
$fs = [System.IO.File]::Create($ico); $w = New-Object System.IO.BinaryWriter $fs
$w.Write([int16]0); $w.Write([int16]1); $w.Write([int16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $s = $sizes[$i]; $sz = if ($s -eq 256) { 0 } else { $s }
  $w.Write([byte]$sz); $w.Write([byte]$sz); $w.Write([byte]0); $w.Write([byte]0); $w.Write([int16]1); $w.Write([int16]32); $w.Write([int]$frames[$i].Length); $w.Write([int]$offset)
  $offset += $frames[$i].Length
}
foreach ($f in $frames) { $w.Write([byte[]]$f) }
$w.Close()

# Rust build (rustup's MSVC toolchain). The icon above is embedded by build.rs.
Push-Location $dir
try {
  cargo build --release
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
} finally { Pop-Location }
$out = Join-Path $dir 'T3Presence.exe'
Copy-Item (Join-Path $dir 'target\release\T3Presence.exe') $out -Force
"built $out ($([math]::Round((Get-Item $out).Length / 1KB)) KB)"
