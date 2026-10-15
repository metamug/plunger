Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hgt, bool r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@
Add-Type @'
using System; using System.Text; using System.Runtime.InteropServices;
public class E {
  public delegate bool CB(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(CB cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  public static IntPtr Find(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h,l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p==pid && IsWindowVisible(h)) { var sb=new StringBuilder(256); GetClassName(h, sb, 256); if (sb.ToString()=="Window Class") found = h; }
      return true; }, IntPtr.Zero);
    return found;
  }
}
'@
function Get-Hwnd { [E]::Find([uint32]$script:proc.Id) }
[W]::SetProcessDPIAware() | Out-Null
$script:proc = $null

function Start-App([string]$exe, [string]$dataDir, [int]$w = 1280, [int]$h = 860) {
  $env:PLUNGER_DATA_DIR = $dataDir
  $script:proc = Start-Process -FilePath $exe -PassThru
  for ($i = 0; $i -lt 50; $i++) {
    Start-Sleep -Milliseconds 200
    $script:proc.Refresh()
    if ((Get-Hwnd) -ne [IntPtr]::Zero) { break }
  }
  Start-Sleep -Milliseconds 600
  [W]::MoveWindow((Get-Hwnd), 40, 20, $w, $h, $true) | Out-Null
  Focus-App
  Start-Sleep -Milliseconds 500
}
function Focus-App { $script:proc.Refresh(); [W]::ShowWindow((Get-Hwnd), 9) | Out-Null; [W]::SetForegroundWindow((Get-Hwnd)) | Out-Null; Start-Sleep -Milliseconds 150 }
function Stop-App { if ($script:proc -and -not $script:proc.HasExited) { $script:proc.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 700; if (-not $script:proc.HasExited) { $script:proc.Kill() } } }
function Get-Rect { $script:proc.Refresh(); $r = New-Object W+RECT; [W]::GetWindowRect((Get-Hwnd), [ref]$r) | Out-Null; $r }
function Shot([string]$path) {
  $r = Get-Rect
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $g.Dispose(); $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}
# Coordinates are relative to the window's top-left corner.
function Click([int]$x, [int]$y, [switch]$Right, [switch]$Double) {
  $r = Get-Rect
  [W]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
  Start-Sleep -Milliseconds 120
  $down, $up = if ($Right) { 0x8, 0x10 } else { 0x2, 0x4 }
  $n = if ($Double) { 2 } else { 1 }
  for ($i = 0; $i -lt $n; $i++) { [W]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40; [W]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60 }
  Start-Sleep -Milliseconds 250
}
function Escape-Keys([string]$t) { ($t.ToCharArray() | ForEach-Object { if ('+^%~(){}[]'.Contains($_)) { "{$_}" } else { "$_" } }) -join '' }
function Txt([string]$text) { Focus-App; [System.Windows.Forms.SendKeys]::SendWait((Escape-Keys $text)); Start-Sleep -Milliseconds 250 }
function Keys([string]$combo) { Focus-App; [System.Windows.Forms.SendKeys]::SendWait($combo); Start-Sleep -Milliseconds 300 }
function Wheel([int]$x, [int]$y, [int]$delta) {
  $r = Get-Rect; [W]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
  [W]::mouse_event(0x800, 0, 0, [uint32]$delta, [UIntPtr]::Zero); Start-Sleep -Milliseconds 300
}

if (-not $script:proc) { $script:proc = Get-Process plunger -ErrorAction SilentlyContinue | Select-Object -First 1 }
