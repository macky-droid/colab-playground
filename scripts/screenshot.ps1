<#
.SYNOPSIS
Starts Colab Playground on the mock backend, drives its window and saves screenshots, without
touching your mouse or keyboard. Windows only.

.DESCRIPTION
-Start builds the debug app, copies it to a temporary folder of its own (so cargo can still
rebuild it) and runs the copy with --mock and its own config folder. It prints the process id,
which every other call takes as -Id.

Each later call acts on that window alone, by posting window messages, in this order: -Size,
-Click, -Hover, -Wheel, -Type, -Key, then waits -WaitMs and saves -Out. Coordinates are pixels
of the saved PNG, so read them off the last screenshot. -Stop ends the app and deletes its
temporary folder.

A process -Start did not launch, such as your own Colab Playground, is refused: nothing is sent
to it and it is never stopped.

.EXAMPLE
$id = ./scripts/screenshot.ps1 -Start -AppArgs '--mock-scenario', 'single'
./scripts/screenshot.ps1 -Id $id -Out target/shots/overview.png
./scripts/screenshot.ps1 -Id $id -Click 50, 127 -Out target/shots/sessions.png
./scripts/screenshot.ps1 -Id $id -Size 880, 560 -Wheel 440, 300, 5 -Out target/shots/scrolled.png
./scripts/screenshot.ps1 -Id $id -Stop

.EXAMPLE
$id = ./scripts/screenshot.ps1 -Start -SettingsJson '{"appearance": "dark"}'
#>
[CmdletBinding(DefaultParameterSetName = 'Drive')]
param(
  # Build and start the app; prints the process id.
  [Parameter(ParameterSetName = 'Start', Mandatory)] [switch]$Start,
  # Arguments after --mock, such as '--mock-scenario', 'single' or '--mock-fail', 'ssh-busy'.
  [Parameter(ParameterSetName = 'Start')] [string[]]$AppArgs = @(),
  # settings.json to start with, such as '{"appearance": "dark"}' or '{"language": "en-US"}'.
  [Parameter(ParameterSetName = 'Start')] [string]$SettingsJson,

  # The process id -Start printed.
  [Parameter(ParameterSetName = 'Drive', Mandatory)]
  [Parameter(ParameterSetName = 'Stop', Mandatory)] [int]$Id,
  # Window content size in logical pixels: width, height (the app keeps at least 880, 560).
  [Parameter(ParameterSetName = 'Drive')] [ValidateCount(2, 2)] [int[]]$Size,
  # A left click: x, y.
  [Parameter(ParameterSetName = 'Drive')] [ValidateCount(2, 2)] [int[]]$Click,
  # Move the pointer there and leave it: x, y.
  [Parameter(ParameterSetName = 'Drive')] [ValidateCount(2, 2)] [int[]]$Hover,
  # Wheel turns over a point: x, y, notches (positive scrolls down).
  [Parameter(ParameterSetName = 'Drive')] [ValidateCount(3, 3)] [int[]]$Wheel,
  # Text typed into the focused field.
  [Parameter(ParameterSetName = 'Drive')] [string]$Type,
  # A key pressed in the window, such as Escape to close a dialog.
  [Parameter(ParameterSetName = 'Drive')]
  [ValidateSet('Enter', 'Escape', 'Tab', 'Backspace', 'Delete', 'Left', 'Right', 'Up', 'Down',
    'Home', 'End', 'PageUp', 'PageDown')] [string]$Key,
  # How many times -Key is pressed.
  [Parameter(ParameterSetName = 'Drive')] [ValidateRange(1, 100)] [int]$KeyRepeat = 1,
  # Where to save the PNG; folders are created.
  [Parameter(ParameterSetName = 'Drive')] [string]$Out,
  # How long the app gets to draw before the capture.
  [Parameter(ParameterSetName = 'Drive')] [int]$WaitMs = 700,

  # End the app and delete its temporary folder.
  [Parameter(ParameterSetName = 'Stop', Mandatory)] [switch]$Stop
)
$ErrorActionPreference = 'Stop'
$runs = Join-Path ([IO.Path]::GetTempPath()) 'colab-playground-screenshots'

# A copy -Start launched runs as <runs>\<its folder>\colab-playground.exe. Anything else, such
# as your own Colab Playground, could be signed in to a real account.
function Get-RunFolder([System.Diagnostics.Process]$process) {
  $exe = $process.Path
  if ($exe) {
    $folder = [IO.Path]::GetDirectoryName($exe)
    if ([IO.Path]::GetFileName($exe) -eq 'colab-playground.exe' -and
        [string]::Equals([IO.Path]::GetDirectoryName($folder), $runs, [StringComparison]::OrdinalIgnoreCase)) {
      return $folder
    }
  }
  throw "process $($process.Id) was not started by -Start"
}

if ($Start) {
  $manifest = Join-Path (Split-Path $PSScriptRoot -Parent) 'Cargo.toml'
  $exe = $null
  # The executable's path comes from cargo, so a custom target directory works too.
  & cargo build -p colab-playground --manifest-path $manifest --message-format=json-render-diagnostics |
    ForEach-Object {
      if ($_ -like '{*') {
        $message = $_ | ConvertFrom-Json
        if ($message.reason -eq 'compiler-artifact' -and $message.executable -and
            $message.target.name -eq 'colab-playground') { $exe = $message.executable }
      }
    }
  if ($LASTEXITCODE -ne 0 -or -not $exe) { throw 'cargo build failed' }
  New-Item -ItemType Directory -Force $runs | Out-Null
  # Without -Force, so two starts never share a folder.
  $run = Join-Path $runs ('{0}-{1}' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), [guid]::NewGuid().ToString('N').Substring(0, 8))
  New-Item -ItemType Directory $run | Out-Null
  $process = $null
  try {
    $config = Join-Path $run 'config'
    New-Item -ItemType Directory $config | Out-Null
    Copy-Item $exe (Join-Path $run 'colab-playground.exe')
    # Without a byte order mark, which the app's JSON parser refuses.
    if ($SettingsJson) { [IO.File]::WriteAllText((Join-Path $config 'settings.json'), $SettingsJson) }
    # Start-Process joins its arguments with spaces; quoted the way Windows splits a command
    # line, each stays one argument.
    $arguments = (@('--mock') + $AppArgs | ForEach-Object {
        if ($_ -and $_ -notmatch '[\s"]') { return $_ }
        '"' + ($_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
      }) -join ' '
    $before = $env:COLAB_PLAYGROUND_CONFIG_DIR
    $env:COLAB_PLAYGROUND_CONFIG_DIR = $config
    try {
      $process = Start-Process -FilePath (Join-Path $run 'colab-playground.exe') -ArgumentList $arguments -PassThru
    } finally {
      $env:COLAB_PLAYGROUND_CONFIG_DIR = $before
    }
    for ($i = 0; $i -lt 100 -and $process.MainWindowHandle -eq 0; $i++) {
      if ($process.HasExited) { throw "the app exited with code $($process.ExitCode)" }
      Start-Sleep -Milliseconds 200
      $process.Refresh()
    }
    if ($process.MainWindowHandle -eq 0) { throw 'the app opened no window' }
    # Every later call makes this check, so a copy it would refuse fails here instead.
    [void](Get-RunFolder $process)
  } catch {
    # Leave nothing that no process id points to.
    if ($process -and -not $process.HasExited) {
      $process | Stop-Process
      [void]$process.WaitForExit(10000)
    }
    Remove-Item -LiteralPath $run -Recurse -Force -ErrorAction SilentlyContinue
    throw
  }
  Start-Sleep -Milliseconds 1000
  Write-Output $process.Id
  return
}

$process = Get-Process -Id $Id
$folder = Get-RunFolder $process

if ($Stop) {
  $process | Stop-Process
  if (-not $process.WaitForExit(10000)) { throw "process $Id is still running, so $folder is left in place" }
  Remove-Item -LiteralPath $folder -Recurse -Force
  return
}

Add-Type -AssemblyName System.Drawing
if (-not ('ColabPlaygroundShot' -as [type])) {
  Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ColabPlaygroundShot {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  // The W form: through the A form, WM_CHAR would carry a code-page byte, not UTF-16.
  [DllImport("user32.dll", EntryPoint = "PostMessageW")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
}
$win = [ColabPlaygroundShot]
# Physical pixels everywhere, as in the PNG.
[void]$win::SetProcessDPIAware()
$h = $process.MainWindowHandle
if ($h -eq 0) { throw "process $Id has no window" }

function Pack([int]$x, [int]$y) { [IntPtr]((($y -band 0xFFFF) -shl 16) -bor ($x -band 0xFFFF)) }
function Post([uint32]$message, [IntPtr]$wParam, [IntPtr]$lParam) {
  if (-not $win::PostMessage($h, $message, $wParam, $lParam)) {
    throw ('message 0x{0:X4} was not posted: the window may have closed' -f $message)
  }
}

if ($Size) {
  $scale = $win::GetDpiForWindow($h) / 96.0
  $outer = New-Object ColabPlaygroundShot+RECT; $inner = New-Object ColabPlaygroundShot+RECT
  [void]$win::GetWindowRect($h, [ref]$outer); [void]$win::GetClientRect($h, [ref]$inner)
  $frameW = ($outer.R - $outer.L) - ($inner.R - $inner.L)
  $frameH = ($outer.B - $outer.T) - ($inner.B - $inner.T)
  $width = [int]($Size[0] * $scale) + $frameW; $height = [int]($Size[1] * $scale) + $frameH
  # No move, z-order change or activation.
  if (-not $win::SetWindowPos($h, [IntPtr]::Zero, 0, 0, $width, $height, 0x0002 -bor 0x0004 -bor 0x0010)) {
    throw 'the window was not resized: it may have closed'
  }
  Start-Sleep -Milliseconds 400
}
if ($Click) {
  $at = Pack $Click[0] $Click[1]
  Post 0x0200 ([IntPtr]0) $at   # WM_MOUSEMOVE
  Start-Sleep -Milliseconds 80
  Post 0x0201 ([IntPtr]1) $at   # WM_LBUTTONDOWN
  Start-Sleep -Milliseconds 60
  Post 0x0202 ([IntPtr]0) $at   # WM_LBUTTONUP
}
if ($Hover) {
  Post 0x0200 ([IntPtr]0) (Pack $Hover[0] $Hover[1])
}
if ($Wheel) {
  Post 0x0200 ([IntPtr]0) (Pack $Wheel[0] $Wheel[1])
  Start-Sleep -Milliseconds 60
  $delta = if ($Wheel[2] -gt 0) { -120 } else { 120 }
  for ($i = 0; $i -lt [Math]::Abs($Wheel[2]); $i++) {
    # WM_MOUSEWHEEL carries screen coordinates, taken for each notch in case the window moved.
    $point = New-Object ColabPlaygroundShot+POINT; $point.X = $Wheel[0]; $point.Y = $Wheel[1]
    [void]$win::ClientToScreen($h, [ref]$point)
    Post 0x020A ([IntPtr](($delta -band 0xFFFF) -shl 16)) (Pack $point.X $point.Y)
    Start-Sleep -Milliseconds 60
  }
}
if ($Type) {
  foreach ($c in $Type.ToCharArray()) {
    Post 0x0102 ([IntPtr][int]$c) ([IntPtr]1)   # WM_CHAR
    Start-Sleep -Milliseconds 20
  }
}
if ($Key) {
  $vk = @{ Enter = 0x0D; Escape = 0x1B; Tab = 0x09; Backspace = 0x08; Delete = 0x2E; Left = 0x25
    Right = 0x27; Up = 0x26; Down = 0x28; Home = 0x24; End = 0x23; PageUp = 0x21; PageDown = 0x22 }[$Key]
  for ($i = 0; $i -lt $KeyRepeat; $i++) {
    Post 0x0100 ([IntPtr]$vk) ([IntPtr]1)            # WM_KEYDOWN
    Post 0x0101 ([IntPtr]$vk) ([IntPtr]0xC0000001)   # WM_KEYUP
    Start-Sleep -Milliseconds 40
  }
}
Start-Sleep -Milliseconds $WaitMs
if ($Out) {
  $path = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
  New-Item -ItemType Directory -Force (Split-Path $path -Parent) | Out-Null
  $rect = New-Object ColabPlaygroundShot+RECT
  [void]$win::GetClientRect($h, [ref]$rect)
  $bitmap = New-Object System.Drawing.Bitmap ($rect.R - $rect.L), ($rect.B - $rect.T)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  try {
    $dc = $graphics.GetHdc()
    # PW_CLIENTONLY | PW_RENDERFULLCONTENT: what DirectX presents, even if covered.
    $printed = $win::PrintWindow($h, $dc, 3)
    $graphics.ReleaseHdc($dc)
    # A failed capture would save a blank image over the last good one.
    if (-not $printed) { throw 'the window was not captured: it may have closed' }
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  } finally {
    $graphics.Dispose()
    $bitmap.Dispose()
  }
  Write-Host "saved $path ($($rect.R - $rect.L)x$($rect.B - $rect.T))"
}
