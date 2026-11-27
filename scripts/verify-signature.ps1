<#
Lumen Windows 签名校验脚本

用法：
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-signature.ps1
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-signature.ps1 -Path "src-tauri\target\release\bundle\nsis"
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-signature.ps1 -Path "a.exe","b.exe" -AllowUnsigned

不传 -Path 时默认检查 NSIS 产物目录下的安装包。
-AllowUnsigned：仅报告状态，未签名也返回 0（用于当前无证书阶段）。
默认严格模式：存在未签名或签名无效的文件时返回非 0。
#>
param(
  [string[]]$Path = @("src-tauri\target\release\bundle\nsis"),
  [switch]$AllowUnsigned
)

$ErrorActionPreference = 'Stop'

# npm 等环境可能把 PowerShell 7 的模块目录放进 PSModulePath，
# 导致 Windows PowerShell 5.1 自动加载 Security 模块失败；这里显式加载系统模块。
# PowerShell 7 自带 Get-AuthenticodeSignature，强行导入 5.1 模块会因命令重名而失败。
$securityModule = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1'
if ($PSVersionTable.PSVersion.Major -lt 7 -and (Test-Path $securityModule)) {
  Import-Module $securityModule -Force
}

$files = @()

foreach ($item in $Path) {
  if (Test-Path $item -PathType Container) {
    $files += Get-ChildItem $item -Filter *.exe -File
  } elseif (Test-Path $item) {
    $files += Get-Item $item
  } else {
    Write-Warning "路径不存在：$item"
  }
}

if ($files.Count -eq 0) {
  throw '没有找到可校验的 exe 文件。'
}

$bad = 0
foreach ($file in $files) {
  $sig = Get-AuthenticodeSignature -FilePath $file.FullName
  if ($sig.Status -eq 'Valid') {
    $subject = $sig.SignerCertificate.Subject
    Write-Host "[通过] $($file.Name) -> $subject"
  } else {
    $bad += 1
    Write-Warning "[未签名或无效] $($file.Name) -> $($sig.Status)"
  }
}

if ($bad -gt 0) {
  if ($AllowUnsigned) {
    Write-Host "共 $($files.Count) 个文件，其中 $bad 个未签名（-AllowUnsigned 模式，视为通过）。"
  } else {
    Write-Error "共 $($files.Count) 个文件，其中 $bad 个未签名或签名无效。"
    exit 1
  }
} else {
  Write-Host "共 $($files.Count) 个文件签名均有效。"
}
