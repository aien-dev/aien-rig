# End-to-end checks of rig-tune on a real Windows machine (CI: windows-latest,
# no NVIDIA GPU). Run from the tuner/ directory after `cargo build --release`.
# Writes everything it produces to $env:AIEN_RIG_HOME so CI can upload it.
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path 'target\release\rig-tune.exe').Path
$home_ = $env:AIEN_RIG_HOME
New-Item -ItemType Directory -Force -Path $home_ | Out-Null
function Fail($msg) { Write-Host "FAIL: $msg"; exit 1 }

Write-Host '== 1. scan runs and returns findings'
& $exe scan --json --quick | Out-File -Encoding utf8 "$home_\scan.json"
if ($LASTEXITCODE -ne 0) { Fail 'scan exited non-zero' }
$scan = Get-Content "$home_\scan.json" -Raw | ConvertFrom-Json
if ($scan.findings.Count -lt 5) { Fail "scan returned only $($scan.findings.Count) findings" }
& $exe scan --quick
Write-Host "scan: $($scan.findings.Count) findings, $($scan.facts.displays.Count) displays, $($scan.facts.gpus.Count) gpus"

Write-Host '== 2. ETW frame counter vs a program presenting at a known rate'
$probeExe = Join-Path $home_ 'present-probe.exe'
Copy-Item $exe $probeExe -Force
$probeOut = Join-Path $home_ 'probe.json'
$target = 120
$probe = Start-Process -FilePath $probeExe -ArgumentList @('probe-present', '--seconds', '30', '--fps', "$target", '--out', $probeOut) -PassThru -WindowStyle Normal
Start-Sleep -Seconds 4
& $exe measure present-probe.exe --seconds 15 --label ci --game 'CI present probe' --wait 30 --json | Tee-Object -FilePath "$home_\measure.json"
if ($LASTEXITCODE -ne 0) { Fail 'measure exited non-zero' }
$probe.WaitForExit()
$probeResult = Get-Content $probeOut -Raw | ConvertFrom-Json
$receiptFile = Get-ChildItem "$home_\receipts" -Filter 'ci-present-probe-*.json' | Select-Object -First 1
if (-not $receiptFile) { Fail 'no receipt written' }
$r = Get-Content $receiptFile.FullName -Raw | ConvertFrom-Json
$err = [math]::Abs($r.stats.avg_fps - $probeResult.fps) / $probeResult.fps
Write-Host ("probe presented {0:N2} fps (target {1}); rig-tune measured {2:N2} fps avg, 1% low {3:N2}, from {4} event {5}, {6} frames; relative difference {7:P2}" -f `
    $probeResult.fps, $target, $r.stats.avg_fps, $r.stats.low_1pct_fps, $r.frame_source, $r.source_event_id, $r.stats.frames, $err)
if ($r.frame_source -ne 'dxgi') { Fail "expected DXGI present events, got $($r.frame_source)" }
if ($r.stats.frames -lt 1000) { Fail "too few frames: $($r.stats.frames)" }
if ($err -gt 0.03) { Fail "measured FPS is more than 3% away from what the probe presented" }

Write-Host '== 3. apply then undo restores every setting exactly'
$gb = 'HKCU:\Software\Microsoft\GameBar'
$dvr = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\GameDVR'
$dx = 'HKCU:\Software\Microsoft\DirectX\UserGpuPreferences'
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
foreach ($k in @($gb, $dvr, $dx)) { New-Item -Path $k -Force | Out-Null }
# Put the PC in a "needs tuning" state first so every tweak has work to do.
Set-ItemProperty -Path $gb -Name AutoGameModeEnabled -Value 0 -Type DWord
Set-ItemProperty -Path $dvr -Name HistoricalCaptureEnabled -Value 1 -Type DWord
Set-ItemProperty -Path $dx -Name DirectXUserGlobalSettings -Value 'VRROptimizeEnable=0;' -Type String
Remove-ItemProperty -Path $gd -Name HwSchMode -ErrorAction SilentlyContinue
# Power saver, so the power-plan tweak has work to do (falls back to Balanced
# if this machine does not offer Power saver).
powercfg /setactive a1841308-3bce-4d2e-b8c3-6b0e5ee2d0d5
if ($LASTEXITCODE -ne 0) { powercfg /setactive 381b4222-f694-41f0-9685-ff5bb260df2e }
$global:LASTEXITCODE = 0

function Snapshot {
    [ordered]@{
        GameMode = (Get-ItemProperty -Path $gb).AutoGameModeEnabled
        BackgroundRecording = (Get-ItemProperty -Path $dvr).HistoricalCaptureEnabled
        DxGlobal = (Get-ItemProperty -Path $dx).DirectXUserGlobalSettings
        Hags = (Get-ItemProperty -Path $gd -ErrorAction SilentlyContinue).HwSchMode
        Power = (powercfg /getactivescheme)
    } | ConvertTo-Json -Compress
}
$before = Snapshot
Write-Host "before: $before"
& $exe apply --yes --only 'hags-on,game-mode-on,background-recording-off,windowed-optimizations-on,power-plan-high'
if ($LASTEXITCODE -ne 0) { Fail 'apply exited non-zero' }
$after = Snapshot
Write-Host "after:  $after"
$a = $after | ConvertFrom-Json
if ($a.GameMode -ne 1) { Fail 'game mode not turned on' }
if ($a.BackgroundRecording -ne 0) { Fail 'background recording not turned off' }
if ($a.DxGlobal -ne 'VRROptimizeEnable=0;SwapEffectUpgradeEnable=1;') { Fail "windowed optimizations wrong: $($a.DxGlobal)" }
if ($a.Hags -ne 2) { Fail 'HAGS not set' }
$b = $before | ConvertFrom-Json
if ($b.Power -match 'a1841308' -and $a.Power -match 'a1841308') { Fail 'still on Power saver after apply' }
& $exe undo --list
& $exe undo --yes
if ($LASTEXITCODE -ne 0) { Fail 'undo exited non-zero' }
$restored = Snapshot
Write-Host "undone: $restored"
if ($restored -ne $before) { Fail 'undo did not restore the exact old values' }
& $exe undo --list

Write-Host '== 4. game mode pauses and resumes a stand-in server process'
$fake = Join-Path $home_ 'aien-rig-server.exe'
Copy-Item $exe $fake -Force
$srv = Start-Process -FilePath $fake -ArgumentList @('probe-present', '--seconds', '60', '--fps', '30') -PassThru
Start-Sleep -Seconds 3
& $exe game-mode on
$t1 = (Get-Process -Id $srv.Id).TotalProcessorTime
Start-Sleep -Seconds 3
$t2 = (Get-Process -Id $srv.Id).TotalProcessorTime
if ($t2 -ne $t1) { Fail "paused server still used CPU ($t1 -> $t2)" }
& $exe game-mode status
& $exe game-mode off
Start-Sleep -Seconds 2
$t3 = (Get-Process -Id $srv.Id).TotalProcessorTime
if ($t3 -le $t2) { Fail 'server did not resume' }
Stop-Process -Id $srv.Id -Force
Write-Host "game mode: CPU time frozen at $t1 while paused, $t3 after resume"

Write-Host 'ALL END-TO-END CHECKS PASSED'
