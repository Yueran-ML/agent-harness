# Preserve the source clip, slow its final three seconds, and remove its audio.
# Requires ffmpeg and ffprobe on PATH. The source and output live outside Git.
param([string]$AssetDirectory = $env:COLUMBINA_ASSET_DIR)
$ErrorActionPreference = 'Stop'
$ffmpeg = (Get-Command ffmpeg -ErrorAction Stop).Source
$ffprobe = (Get-Command ffprobe -ErrorAction Stop).Source
if (-not $AssetDirectory) {
    $localConfig = Join-Path $PSScriptRoot '../assets.local.json'
    if (Test-Path -LiteralPath $localConfig) {
        $AssetDirectory = (Get-Content -Raw -LiteralPath $localConfig | ConvertFrom-Json).assetDirectory
    }
}
if (-not $AssetDirectory -or -not [IO.Path]::IsPathRooted($AssetDirectory)) {
    throw 'Set COLUMBINA_ASSET_DIR or assets.local.json to an external artwork directory. See ASSETS.md.'
}
$sourcePath = Join-Path $AssetDirectory 'columbina-intro.mp4'
$outputPath = Join-Path $AssetDirectory 'columbina-intro-extended.mp4'
$source = (& $ffprobe -v error -show_entries format=duration:format_tags:stream=codec_type -of json $sourcePath) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the opening video.' }
$requiredTags = @('AIGC', 'UserComment', 'LvMetaInfo')
foreach ($tag in $requiredTags) {
    if (-not $source.format.tags.$tag) { throw "The source is missing its required $tag provenance tag." }
}
$culture = [Globalization.CultureInfo]::InvariantCulture
$duration = [double]::Parse($source.format.duration, $culture)
$tailDuration = 3.0
$tailSpeed = 0.5
if ($duration -le $tailDuration) { throw 'The source clip is too short for the three-second ending.' }
$cut = ($duration - $tailDuration).ToString('0.######', $culture)
$end = $duration.ToString('0.######', $culture)
$extended = ($duration - $tailDuration + $tailDuration / $tailSpeed).ToString('0.######', $culture)
$filter = @"
[0:v:0]split=2[vhead][vtail];
[vhead]trim=end=$cut,setpts=PTS-STARTPTS[vnormal];
[vtail]trim=start=${cut}:end=$end,setpts=(PTS-STARTPTS)/0.5[vslow];
[vnormal][vslow]concat=n=2:v=1:a=0,tpad=stop_mode=clone:stop_duration=0.3,trim=duration=$extended,format=yuv420p[vout]
"@
& $ffmpeg -hide_banner -y -i $sourcePath -filter_complex $filter -map '[vout]' -an `
    -c:v libx264 -preset medium -crf 18 -fps_mode vfr -map_metadata 0 -movflags +faststart+use_metadata_tags $outputPath
if ($LASTEXITCODE -ne 0) { throw 'The opening video extension failed.' }
$output = (& $ffprobe -v error -show_entries format=duration,size:format_tags:stream=codec_type,codec_name,width,height -of json $outputPath) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the extended opening video.' }
foreach ($tag in $requiredTags) {
    if ($output.format.tags.$tag -cne $source.format.tags.$tag) {
        throw "The extended video did not preserve its $tag provenance tag. Do not distribute this output."
    }
}
if (@($output.streams | Where-Object codec_type -eq 'audio').Count -ne 0) {
    throw 'The extended opening video still contains audio.'
}
Write-Output "Extended video: $($output.format.duration) seconds; no audio; AIGC, UserComment and LvMetaInfo preserved."
