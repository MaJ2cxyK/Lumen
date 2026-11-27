<#
Lumen 自动更新签名密钥生成脚本

用法：
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/gen-updater-key.ps1

可选参数：
  -Out       私钥输出路径，默认 .keys\lumen-updater.key
  -Password  私钥密码；不传则交互式输入

生成后：
  1. 私钥文件（含 .pub 公钥）放在仓库外或 .gitignore 中；
  2. 把 .pub 的内容填入 src-tauri/tauri.release.conf.json 的 plugins.updater.pubkey；
  3. 发布时设置 TAURI_SIGNING_PRIVATE_KEY（私钥内容或路径）与 TAURI_SIGNING_PRIVATE_KEY_PASSWORD。
#>
param(
  [string]$Out = ".keys\lumen-updater.key",
  [string]$Password
)

$ErrorActionPreference = 'Stop'

if (Test-Path $Out) {
  throw "密钥已存在：$Out。如需重新生成请先备份并删除，或使用 -Out 指定新路径。"
}

$dir = Split-Path -Parent $Out
if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }

if ($Password) {
  npx tauri signer generate -w $Out -p $Password
} else {
  Write-Host '未提供 -Password，将按提示输入私钥密码。'
  npx tauri signer generate -w $Out
}

if ($LASTEXITCODE -ne 0) {
  throw "密钥生成失败，退出码 $LASTEXITCODE"
}

if (Test-Path "$Out.pub") {
  Write-Host ''
  Write-Host '公钥（填入 tauri.release.conf.json 的 plugins.updater.pubkey）：'
  Get-Content "$Out.pub"
}
