<#
Lumen CI 发布脚本（签名 + updater 产物 + latest.json）

用法：
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-ci.ps1
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-ci.ps1 -SkipSign -SkipVerify
  # -SkipSign 为纯构建演练：不合并签名配置、不生成 updater 产物与 latest.json
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-ci.ps1 -SkipCodeSign
  # -SkipCodeSign：无 Authenticode 证书时使用；仍生成 updater 签名产物与 latest.json

前置环境变量：
  LUMEN_SIGN_PFX_BASE64 或 LUMEN_SIGN_PFX / LUMEN_SIGN_THUMBPRINT  代码签名证书
  LUMEN_SIGN_PFX_PASSWORD                                           证书密码（PFX 方式）
  TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD    updater 私钥（自动继承给构建）
  TAURI_SIGNING_PUBLIC_KEY                                          updater 公钥（写入发布配置）
  UPDATE_BASE_URL                                                   更新包下载前缀（可选）

流程：
  1. node scripts/prepare-publish-config.mjs  生成 src-tauri/tauri.publish.conf.json
  2. npx tauri build --config src-tauri/tauri.publish.conf.json --bundles nsis
  3. scripts/verify-signature.ps1 严格校验安装包签名
  4. node scripts/gen-latest-json.mjs          生成 bundle/nsis/latest.json
#>
param(
  [switch]$SkipSign,
  [switch]$SkipCodeSign,
  [switch]$SkipVerify
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$bundleDir = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$publishConf = 'src-tauri/tauri.publish.conf.json'
$skipCodeSignRequested = ([bool]$SkipCodeSign) -or ($env:SKIP_CODE_SIGN -match '^(1|true|yes)$')

# CI secret 通常命名为 WINDOWS_CERT_*，这里映射到签名脚本实际使用的变量
if (-not $env:LUMEN_SIGN_PFX_BASE64 -and $env:WINDOWS_CERT_PFX_BASE64) {
  $env:LUMEN_SIGN_PFX_BASE64 = $env:WINDOWS_CERT_PFX_BASE64
}
if (-not $env:LUMEN_SIGN_PFX_PASSWORD -and $env:WINDOWS_CERT_PFX_PASSWORD) {
  $env:LUMEN_SIGN_PFX_PASSWORD = $env:WINDOWS_CERT_PFX_PASSWORD
}

if (-not $SkipSign -and -not $env:TAURI_SIGNING_PRIVATE_KEY) {
  throw '未设置 TAURI_SIGNING_PRIVATE_KEY，无法生成可用的自动更新签名。'
}

Push-Location $root
try {
  # tag 发布时校验版本号，避免 v0.2.0 标签产出 0.1.0 的清单
  $tag = $env:GITHUB_REF_NAME
  if ($tag -and $tag.StartsWith('v')) {
    $version = (Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
    if ("v$version" -ne $tag) {
      throw "标签 $tag 与 tauri.conf.json 中的版本 v$version 不一致，请先更新版本号。"
    }
  }

  # 演练模式不合并 signing 配置，构建时不会调用 sign-windows.ps1
  if ($SkipSign) {
    $env:SKIP_SIGN = '1'
    Remove-Item Env:\SKIP_CODE_SIGN -ErrorAction SilentlyContinue
  } elseif ($skipCodeSignRequested) {
    Remove-Item Env:\SKIP_SIGN -ErrorAction SilentlyContinue
    $env:SKIP_CODE_SIGN = '1'
  } else {
    Remove-Item Env:\SKIP_SIGN -ErrorAction SilentlyContinue
    Remove-Item Env:\SKIP_CODE_SIGN -ErrorAction SilentlyContinue
  }

  Write-Host '=== 1/3 生成发布配置 ==='
  node scripts/prepare-publish-config.mjs
  if ($LASTEXITCODE -ne 0) {
    throw "生成发布配置失败，退出码 $LASTEXITCODE"
  }

  Write-Host '=== 2/3 构建（NSIS + updater 产物）==='
  # 不覆盖 TAURI_SIGNING_* 环境变量，构建进程直接继承。
  # --no-binary-patching：只发 NSIS，避免打包阶段二次补丁覆盖主程序签名。
  npx tauri build --config $publishConf --no-binary-patching --bundles nsis
  if ($LASTEXITCODE -ne 0) {
    throw "构建失败，退出码 $LASTEXITCODE"
  }
} finally {
  Pop-Location
}

if (-not (Test-Path $bundleDir)) {
  throw "找不到 NSIS 产物目录：$bundleDir"
}

Write-Host '=== 3/3 校验签名并生成 latest.json ==='
if ($SkipVerify -or $SkipSign) {
  Write-Warning '已跳过安装包签名校验（演练模式），产物不可用于正式发布。'
} elseif ($skipCodeSignRequested) {
  & (Join-Path $PSScriptRoot 'verify-signature.ps1') -Path $bundleDir -AllowUnsigned
  if ($LASTEXITCODE -ne 0) {
    throw "安装包状态检查失败，退出码 $LASTEXITCODE"
  }
  Write-Warning '安装包未进行 Authenticode 代码签名（无证书模式）；Windows SmartScreen 可能提示风险。'
} else {
  & (Join-Path $PSScriptRoot 'verify-signature.ps1') -Path $bundleDir
  if ($LASTEXITCODE -ne 0) {
    throw "安装包签名校验未通过，退出码 $LASTEXITCODE"
  }
}

if ($SkipSign) {
  Write-Warning '演练模式未生成最新更新清单（latest.json）。'
} else {
  node scripts/gen-latest-json.mjs
  if ($LASTEXITCODE -ne 0) {
    throw "生成 latest.json 失败，退出码 $LASTEXITCODE"
  }
}

Write-Host "发布产物目录：$bundleDir"
