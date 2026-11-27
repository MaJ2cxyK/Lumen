<#
Lumen 发布脚本（自动更新产物 + latest.json，不负责上传）

前置：
  1. 已运行 scripts/gen-updater-key.ps1 生成密钥；
  2. 已把公钥填入 src-tauri/tauri.release.conf.json 的 plugins.updater.pubkey；
  3. 已设置 TAURI_SIGNING_PRIVATE_KEY（私钥文件路径或内容）与
     TAURI_SIGNING_PRIVATE_KEY_PASSWORD；
  4. 已设置 -BaseUrl 为安装包最终可下载地址的前缀（不含文件名）。

用法：
  $env:TAURI_SIGNING_PRIVATE_KEY = "D:\keys\lumen-updater.key"
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "***"
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release.ps1 -BaseUrl https://updates.example.com/lumen -Notes "修复..."

产出：
  release/Lumen_<version>_x64-setup.exe
  release/Lumen_<version>_x64-setup.nsis.zip
  release/Lumen_<version>_x64-setup.nsis.zip.sig
  release/latest.json
#>
param(
  [string]$Version,
  [string]$Notes = "",
  [string]$BaseUrl,
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$confPath = Join-Path $root 'src-tauri\tauri.conf.json'
$releaseConf = Join-Path $root 'src-tauri\tauri.release.conf.json'
$bundleDir = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$outDir = Join-Path $root 'release'

$conf = Get-Content $confPath -Raw | ConvertFrom-Json
if (-not $Version) { $Version = $conf.version }
if (-not $BaseUrl) {
  throw '请通过 -BaseUrl 提供更新包下载地址前缀，例如 https://updates.example.com/lumen'
}
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  throw '未设置 TAURI_SIGNING_PRIVATE_KEY，无法生成可用的自动更新签名。'
}

if (-not $SkipBuild) {
  Write-Host "开始构建 $Version（含自动更新产物）..."
  Push-Location $root
  try {
    npx tauri build --config $releaseConf --bundles nsis
    if ($LASTEXITCODE -ne 0) { throw "构建失败，退出码 $LASTEXITCODE" }
  } finally {
    Pop-Location
  }
}

if (-not (Test-Path $bundleDir)) {
  throw "找不到 NSIS 产物目录：$bundleDir"
}

$installer = Get-ChildItem $bundleDir -Filter '*-setup.exe' -File | Select-Object -First 1
$zip = Get-ChildItem $bundleDir -Filter '*.nsis.zip' -File | Select-Object -First 1
$sig = Get-ChildItem $bundleDir -Filter '*.nsis.zip.sig' -File | Select-Object -First 1

if (-not $installer) { throw '找不到 NSIS 安装包（*-setup.exe）。' }
if (-not $zip) { throw '找不到自动更新包（*.nsis.zip），请确认 tauri.release.conf.json 已启用 createUpdaterArtifacts。' }
if (-not $sig) { throw '找不到更新包签名（*.nsis.zip.sig），请确认签名密钥环境变量已正确设置。' }

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
Copy-Item $installer.FullName $outDir -Force
Copy-Item $zip.FullName $outDir -Force
Copy-Item $sig.FullName $outDir -Force

$signature = (Get-Content $sig.FullName -Raw).Trim()
$url = "$($BaseUrl.TrimEnd('/'))/$($zip.Name)"
$publishedAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

$latest = [ordered]@{
  version = $Version
  notes = $Notes
  pub_date = $publishedAt
  platforms = [ordered]@{
    'windows-x86_64' = [ordered]@{
      signature = $signature
      url = $url
    }
  }
}

$latestPath = Join-Path $outDir 'latest.json'
$latest | ConvertTo-Json -Depth 5 | Set-Content $latestPath -Encoding UTF8

Write-Host "已生成发布目录：$outDir"
Write-Host "安装包：$($installer.Name)"
Write-Host "更新包：$($zip.Name)"
Write-Host "清单：latest.json（url=$url）"
Write-Host '上传 release 目录内容到你的静态分发地址即可；本脚本不会自动上传。'
