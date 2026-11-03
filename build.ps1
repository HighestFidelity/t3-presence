# Builds T3Presence.exe next to this script: csc from .NET Framework 4.8 (ships with Windows), nothing to install.
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
$preview = Resized $logo 256
$preview.Save((Join-Path $gen 'logo.png'), [System.Drawing.Imaging.ImageFormat]::Png)

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

$fw = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319"
$wpf = "$fw\WPF"
$out = Join-Path $dir 'T3Presence.exe'
$args = @('/nologo', '/target:winexe', '/platform:x64', '/optimize+', '/warn:3',
  "/out:$out", "/win32icon:$ico", "/win32manifest:$src\app.manifest",
  "/resource:$src\Window.xaml,Window.xaml", "/resource:$src\remote-state.py,remote-state.py", "/resource:$gen\logo.png,logo.png", "/resource:$ico,T3Presence.ico",
  "/r:$wpf\PresentationFramework.dll", "/r:$wpf\PresentationCore.dll", "/r:$wpf\WindowsBase.dll", "/r:$fw\System.Xaml.dll",
  '/r:System.dll', '/r:System.Core.dll', '/r:System.Windows.Forms.dll', '/r:System.Drawing.dll', '/r:System.Web.Extensions.dll', '/r:Microsoft.CSharp.dll',
  "$src\Data.cs", "$src\Engine.cs", "$src\Ui.cs")
& "$fw\csc.exe" @args
if ($LASTEXITCODE -ne 0) { throw "csc failed ($LASTEXITCODE)" }
"built $out ($([math]::Round((Get-Item $out).Length / 1KB)) KB)"
