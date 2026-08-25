param(
  [string]$FfmpegPath = "D:\CodexProject\MaxGameStudio\.tmp-video-tools\ffmpeg-9.0-essentials_build\bin\ffmpeg.exe",
  [string]$OutputPath = "D:\CodexProject\MaxGameStudio\.tmp-release\MaxGameStudio-v2.4.6-intro.mp4"
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) -Parent
$screenshots = Join-Path $projectRoot "docs\screenshots"
$work = Join-Path $projectRoot ".tmp-video-build"
$ffprobe = Join-Path (Split-Path $FfmpegPath -Parent) "ffprobe.exe"
New-Item -ItemType Directory -Force $work | Out-Null
New-Item -ItemType Directory -Force (Split-Path $OutputPath -Parent) | Out-Null

$segments = @(
  @{ Image="getting-started.png"; Text="你有没有遇到过这种情况：CS2 客户端提示比赛回放已经过期，但右下角仍然能复制一条官匹分享链接；普通下载器不认识它，想管理 Demo，又不想为了一个桌面工具再部署数据库。MaxGameStudio，就是从这个真实问题开始做出来的一套本地工作台。" },
  @{ Image="official-demo-download.png"; Text="把 CS2 分享出来的链接交给软件，它会完成链接识别、下载任务和本地 Demo 管理。数据使用轻量的 SQLite 保存在电脑上，不要求单独安装 PostgreSQL。你也可以监听常用下载目录，把官匹、完美、五 E 或其他来源的 Demo 集中到一个资料库。" },
  @{ Image="demo-analysis.png"; Text="Demo 解析完成以后，第一屏不再是藏得很深的功能入口，而是整场计分板。比分、KDA、ADR、KAST、爆头率、首杀、AWP 击杀和道具伤害都放在一起。每位玩家还会获得 S 到 D 的表现等级，以及优势和下一步优化方向，让你第一眼就知道这一局发生了什么。" },
  @{ Image="analysis-history.png"; Text="已经解析过的比赛会进入本地历史分析。关掉软件再回来，仍然可以直接重新打开最近的结果，不用为了看旧比赛重复跑一次完整解析。基础分析和二 D 回放缓存是两个阶段，第一次打开某场比赛的二 D 回放会稍慢，之后命中缓存就会快很多。" },
  @{ Image="2d-replay-preview.png"; Text="二 D 回放按回合展示双方站位、移动轨迹、击杀连线、射击弹道、投掷物以及烟火区域。左右阵容里的玩家 ID 和地图标记都可以点击，选中以后会同步高亮。你还可以切换全局、仅 A 队或仅 B 队，隐藏另一队的轨迹和事件，专注复盘单一队伍的执行。" },
  @{ Image="round-assessment.png"; Text="回合结束后，软件会根据击杀、死亡、首杀、爆头和下包拆包事件评价这一回合的全部玩家。除了结果，还能看到完整事件时间线：谁打开了包点，谁完成了有效补枪，谁过早出局。这里的单队视角是透明的战术筛选器，不冒充游戏里的真实几何遮挡。" },
  @{ Image="sensitivity-lab.png"; Text="灵敏度实验室内置无需点击的甩枪和连续跟枪测试。它会结合 DPI、当前游戏灵敏度和分辨率，判断速度偏快、偏慢还是相对均衡，并给出可直接用于 CS2 的灵敏度命令、调整百分比和复测区间。软件还能只读识别本地 Steam 账号与 CS2 配置，预填分辨率和灵敏度，不会偷偷修改 CFG。" },
  @{ Image="magnetic-input-lab.png"; Text="磁轴输入实验室关注的是重复触发、按住抖动、A D 重叠和换向延迟。完成测试后会给出触发行程、快速触发按下与抬起的建议起点，并推荐以零点零五到零点一毫米的小步幅重复验证。用于自动反向输入的 Snap Tap、SOCD 等功能，不建议在 CS2 官匹中开启。" },
  @{ Image="montage-workbench.png"; Text="找到值得保留的回合以后，可以把多杀、残局、首杀或自选时间线片段加入录制队列，再进入合辑工作台和 LiteCut 做整理。它不打算替代专业剪辑软件，而是把找片段、录制和粗剪连接起来，让一场 Demo 更快变成可以分享的内容。" },
  @{ Image="settings.png"; Text="软件采用接近苹果桌面应用的简洁视觉，可以跟随系统或按本地时间切换昼夜模式。它不注入 CS2，不提供作弊功能，主要处理你主动选择的 Demo 和本地配置。项目目前免费开源，最新版安装包、完整说明和问题反馈入口都在 GitHub。希望它能帮你少折腾环境，多花时间看懂真正值得复盘的回合。" }
)

Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$synth.SelectVoice("Microsoft Huihui Desktop")
$synth.Rate = 1
$concatLines = New-Object System.Collections.Generic.List[string]

for ($index = 0; $index -lt $segments.Count; $index++) {
  $number = "{0:D2}" -f ($index + 1)
  $wav = Join-Path $work "narration-$number.wav"
  $video = Join-Path $work "segment-$number.mp4"
  $image = Join-Path $screenshots $segments[$index].Image
  if (-not (Test-Path -LiteralPath $image)) { throw "Missing screenshot: $image" }

  $synth.SetOutputToWaveFile($wav)
  $synth.Speak($segments[$index].Text)
  $synth.SetOutputToNull()

  $audioDuration = [double](& $ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 $wav)
  $duration = [Math]::Round($audioDuration + 1.2, 3)
  $fadeOut = [Math]::Max(0, $duration - 0.55)
  $videoFilter = "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,zoompan=z='min(zoom+0.000035,1.035)':d=1:s=1920x1080:fps=30,fade=t=in:st=0:d=0.45,fade=t=out:st=${fadeOut}:d=0.55,format=yuv420p"
  $audioFilter = "loudnorm=I=-16:TP=-1.5:LRA=11,afade=t=in:st=0:d=0.25,apad=pad_dur=1.2"

  & $FfmpegPath -y -hide_banner -loglevel error -loop 1 -i $image -i $wav -t $duration -vf $videoFilter -af $audioFilter -c:v libx264 -preset medium -crf 19 -r 30 -c:a aac -b:a 192k -movflags +faststart $video
  if ($LASTEXITCODE -ne 0) { throw "FFmpeg failed for segment $number" }
  $escapedVideo = $video.Replace("'", "''")
  $concatLines.Add("file '$escapedVideo'")
}

$synth.Dispose()
$concatFile = Join-Path $work "concat.txt"
[System.IO.File]::WriteAllLines($concatFile, $concatLines, [System.Text.UTF8Encoding]::new($false))
& $FfmpegPath -y -hide_banner -loglevel error -f concat -safe 0 -i $concatFile -c copy -movflags +faststart $OutputPath
if ($LASTEXITCODE -ne 0) { throw "FFmpeg concat failed" }

$finalDuration = & $ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 $OutputPath
Get-Item $OutputPath | Select-Object FullName, Length, @{Name="DurationSeconds";Expression={[Math]::Round([double]$finalDuration, 1)}}
