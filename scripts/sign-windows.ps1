<#
Lumen Windows 代码签名脚本（可选流程，默认构建不签名）

用法：
  $env:LUMEN_SIGN_PFX = "D:\certs\lumen.pfx"
  $env:LUMEN_SIGN_PFX_PASSWORD = "证书密码"
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/sign-windows.ps1 -File "安装包路径"

也支持使用证书库中的指纹：
  $env:LUMEN_SIGN_THUMBPRINT = "AB12..."

CI 场景可用 base64 形式提供 PFX（无需把证书落地成文件）：
  $env:LUMEN_SIGN_PFX_BASE64 = "<pfx 文件内容的 base64>"
  $env:LUMEN_SIGN_PFX_PASSWORD = "证书密码"
  脚本会解码到临时 .pfx，签名结束后在 finally 中删除，避免遗留证书文件。

可选环境变量：
  LUMEN_SIGN_TIMESTAMP_URL  时间戳服务，默认 http://timestamp.digicert.com
  LUMEN_SIGN_SHA            摘要算法，默认 sha256

证书来源优先级：LUMEN_SIGN_PFX > LUMEN_SIGN_PFX_BASE64 > LUMEN_SIGN_THUMBPRINT。
未配置任何证书时会直接报错退出，不会产出「看似签名」的假结果。
#>
param(
  [Parameter(Mandatory = $true)][string]$File,
  [string]$TimestampUrl = $env:LUMEN_SIGN_TIMESTAMP_URL,
  [string]$Digest = $env:LUMEN_SIGN_SHA
)

$ErrorActionPreference = 'Stop'

function Find-SignTool {
  $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  $roots = @(
    "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
    "$env:ProgramFiles\Windows Kits\10\bin"
  )
  foreach ($root in $roots) {
    if (-not $root -or -not (Test-Path $root)) { continue }
    $hit = Get-ChildItem $root -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '\\x64\\' } |
      Sort-Object FullName -Descending |
      Select-Object -First 1
    if ($hit) { return $hit.FullName }
  }
  return $null
}

if (-not (Test-Path $File)) {
  throw "找不到待签名文件：$File"
}
if (-not $Digest) { $Digest = 'sha256' }
if (-not $TimestampUrl) { $TimestampUrl = 'http://timestamp.digicert.com' }

$signTool = Find-SignTool
if (-not $signTool) {
  throw '未找到 signtool.exe，请安装 Windows SDK（勾选签名工具）后重试。'
}

$args = @('sign', '/fd', $Digest, '/td', $Digest, '/tr', $TimestampUrl)

# base64 模式会解码出临时 pfx，必须在 finally 中清理，避免证书残留在磁盘上
$tempPfx = $null
try {
  if ($env:LUMEN_SIGN_PFX) {
    if (-not (Test-Path $env:LUMEN_SIGN_PFX)) {
      throw "LUMEN_SIGN_PFX 指向的证书不存在：$env:LUMEN_SIGN_PFX"
    }
    $args += @('/f', $env:LUMEN_SIGN_PFX)
    if ($env:LUMEN_SIGN_PFX_PASSWORD) {
      $args += @('/p', $env:LUMEN_SIGN_PFX_PASSWORD)
    }
  } elseif ($env:LUMEN_SIGN_PFX_BASE64) {
    # secret 常以多行粘贴，先去掉所有空白字符再解码
    $base64 = ($env:LUMEN_SIGN_PFX_BASE64 -replace '\s', '')
    try {
      $pfxBytes = [Convert]::FromBase64String($base64)
    } catch {
      throw "LUMEN_SIGN_PFX_BASE64 不是合法的 base64：$($_.Exception.Message)"
    }
    $tempPfx = Join-Path ([System.IO.Path]::GetTempPath()) ("lumen-sign-" + [guid]::NewGuid().ToString('N') + ".pfx")
    [System.IO.File]::WriteAllBytes($tempPfx, $pfxBytes)
    $args += @('/f', $tempPfx)
    if ($env:LUMEN_SIGN_PFX_PASSWORD) {
      $args += @('/p', $env:LUMEN_SIGN_PFX_PASSWORD)
    }
  } elseif ($env:LUMEN_SIGN_THUMBPRINT) {
    $args += @('/sha1', $env:LUMEN_SIGN_THUMBPRINT)
  } else {
    throw '未配置签名证书。请设置 LUMEN_SIGN_PFX、LUMEN_SIGN_PFX_BASE64 或 LUMEN_SIGN_THUMBPRINT（PFX 方式可配合 LUMEN_SIGN_PFX_PASSWORD）。'
  }

  $args += $File
  Write-Host "使用 $signTool 签名：$File"
  & $signTool @args
  if ($LASTEXITCODE -ne 0) {
    throw "signtool 签名失败，退出码 $LASTEXITCODE"
  }
  Write-Host "签名完成：$File"
} finally {
  if ($tempPfx -and (Test-Path $tempPfx)) {
    Remove-Item $tempPfx -Force -ErrorAction SilentlyContinue
  }
}
