# Records the real Plunger window doing one scene of the demo, at 30 fps, with a visible pointer.
#
#   powershell -File docs/demo-video/record.ps1 -Scene window
#
# The data folder (C:\Temp\demoapp) and the fake tracker on :18096 come from docs/demo-video/README.md.
# Output: C:\Temp\demo\clips\<scene>.mp4 (the window only, 1180x780).
param(
  [Parameter(Mandatory = $true)][string]$Scene,
  [string]$Exe = "D:\projects\metamug-api-tester\target\release\plunger.exe",
  [string]$Data = "C:\Temp\demoapp",
  [string]$Out = "C:\Temp\demo\clips"
)
. "$PSScriptRoot\ui.ps1"
$ffmpeg = (python -c "import imageio_ffmpeg; print(imageio_ffmpeg.get_ffmpeg_exe())")
New-Item -ItemType Directory -Force $Out | Out-Null

# ---- helpers: a pointer that glides, typing like a person, and a clock ------------------------------------
function Glide([int]$x, [int]$y, [int]$ms = 450) {
  $r = Get-Rect
  $p = [System.Windows.Forms.Cursor]::Position
  $tx = $r.L + $x; $ty = $r.T + $y
  $steps = [math]::Max(8, [int]($ms / 16))
  for ($i = 1; $i -le $steps; $i++) {
    $t = $i / $steps; $e = $t * $t * (3 - 2 * $t)          # smoothstep
    [W]::SetCursorPos([int]($p.X + ($tx - $p.X) * $e), [int]($p.Y + ($ty - $p.Y) * $e)) | Out-Null
    Start-Sleep -Milliseconds 16
  }
}
function Press([int]$x, [int]$y, [int]$ms = 450) { Glide $x $y $ms; Start-Sleep -Milliseconds 120; Click $x $y }
function Type-Slow([string]$text, [int]$ms = 55) {
  Focus-App
  foreach ($ch in $text.ToCharArray()) {
    $k = if ('+^%~(){}[]'.Contains($ch)) { "{$ch}" } else { "$ch" }
    [System.Windows.Forms.SendKeys]::SendWait($k)
    Start-Sleep -Milliseconds ($ms + (Get-Random -Minimum -12 -Maximum 18))
  }
}
$script:t0 = $null
function At([double]$sec) {
  $wait = $sec - ((Get-Date) - $script:t0).TotalSeconds
  if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]($wait * 1000)) }
}
function Start-Recording([string]$name, [double]$seconds) {
  $file = Join-Path $Out "$name.mp4"
  $args = "-y -f gdigrab -framerate 30 -offset_x 40 -offset_y 20 -video_size 1180x780 -draw_mouse 1 -i desktop -t $seconds -c:v libx264 -preset ultrafast -crf 14 -pix_fmt yuv420p `"$file`""
  $script:rec = Start-Process -FilePath $ffmpeg -ArgumentList $args -PassThru -WindowStyle Hidden
  Start-Sleep -Milliseconds 700
  $script:t0 = Get-Date
}
function Run-Plunger([string[]]$items) {
  $quoted = $items | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }
  Start-Process -FilePath $Exe -ArgumentList ($quoted -join ' ') -WindowStyle Hidden -Wait
}
function Finish-Recording { if ($script:rec) { $script:rec.WaitForExit(15000) | Out-Null } }

Start-App $Exe $Data 1180 780
Start-Sleep -Milliseconds 2500
Focus-App

switch ($Scene) {
  "window" {
    Start-Recording "window" 24
    At 0.8;  Keys "^t"
    At 1.6;  Press 650 138 600
    At 2.2;  Type-Slow "https://jsonplaceholder.typicode.com/posts?userId=1&_limit=3" 50
    At 7.0;  Press 312 189 500                      # the Params table, filled in from the URL
    At 9.5;  Keys "^{ENTER}"
    At 13.0; Keys "^f"
    At 13.6; Type-Slow "qui" 120
    At 17.0; Keys "{ESC}"
    At 18.0; Press 120 166 700                      # a saved request in a group
    At 22.0; Keys "^t"
    Finish-Recording
  }
  "variables" {
    Start-Recording "variables" 17
    At 1.0;  Press 224 49 600                       # Tools
    At 2.2;  Press 250 126 400                      # Variables
    At 6.0;  Press 558 73 600                       # close it
    At 7.2;  Press 100 166 600                      # a request that needs {{base_url}}
    At 9.0;  Keys "^{ENTER}"                         # refused, and it says which variable
    Finish-Recording
  }
  "agent" {
    Start-Recording "agent" 27
    At 1.0;  Press 40 136 500                       # fold Saved so History is in view
    At 5.0;  Run-Plunger @("send", "Jira: get issue", "--select", '$.fields.summary', "--select", '$.fields.status.name')
    At 11.0; Run-Plunger @("send", "Jira: search", "--select", '$.total')
    At 18.0; Run-Plunger @("send", "--url", "{{jira_base}}/issue/{{key}}/transitions", "-H", "Authorization: Bearer {{jira_key}}", "--select", '$.transitions[*].name')
    Finish-Recording
  }
  "workflow" {
    Start-Recording "workflow" 16
    At 1.0;  Press 95 670 700                       # the workflow in the sidebar
    At 3.5;  Keys "^+r"                              # run it
    Finish-Recording
  }
  default { throw "unknown scene $Scene" }
}
Stop-App
