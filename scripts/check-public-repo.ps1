[CmdletBinding()]
param(
    [switch]$History
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

$pathRules = @(
    @{ Name = 'local secret directory'; Pattern = '(^|/)(\.keys|\.certs|\.browser|\.trae|\.codex|\.vscode|\.idea|\.fleet|\.history)(/|$)' },
    @{ Name = 'environment file'; Pattern = '(^|/)\.env($|\.)' },
    @{ Name = 'private release config'; Pattern = '(^|/)tauri\.publish\.conf\.json$' },
    @{ Name = 'generated updater manifest'; Pattern = '(^|/)latest\.json$' },
    @{ Name = 'private key or certificate'; Pattern = '\.(pfx|p12|pem|key|crt|cer|der|p8|jks|keystore|snk|kdbx|gpg)$' },
    @{ Name = 'credential file'; Pattern = '(^|/)(id_rsa|id_ed25519|credentials|secrets|\.npmrc|\.pypirc|\.cargo/credentials)(\.|/|$)' },
    @{ Name = 'runtime database or sensitive trace'; Pattern = '\.(db|sqlite|sqlite3|db-wal|db-shm|log|har|dmp|dump|bak|tmp|temp)$' },
    @{ Name = 'build or dependency output'; Pattern = '(^|/)(node_modules|dist|release|target|artifacts|coverage|\.nyc_output)(/|$)' },
    @{ Name = 'binary or release archive'; Pattern = '\.(exe|msi|msix|appx|dmg|pkg|deb|rpm|appimage|snap|7z|rar)$|\.nsis\.zip$' },
    @{ Name = 'internal report'; Pattern = '(^|/)(.*审计报告.*\.md|audit-report.*\.md|security-audit.*\.md|.*\.internal\.md|internal-docs/|audit-reports/)' },
    @{ Name = 'local design reference'; Pattern = '(^|/)(参考图|参考截图|参考资料|design-refs|moodboard)(/|$)' }
)

$contentRules = @(
    @{ Name = 'private key material'; Pattern = '-----BEGIN (RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----|-----BEGIN PGP PRIVATE KEY BLOCK-----|PuTTY-User-Key-File|BEGIN SSH2 ENCRYPTED PRIVATE KEY' },
    @{ Name = 'GitHub token'; Pattern = 'gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}' },
    @{ Name = 'AWS access key'; Pattern = 'AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}' },
    @{ Name = 'API token'; Pattern = 'sk-[A-Za-z0-9_-]{20,}|sk-ant-[A-Za-z0-9_-]{20,}|AIza[0-9A-Za-z_-]{30,}|glpat-[A-Za-z0-9_-]{20,}|xox[abp]-[A-Za-z0-9_-]{20,}' },
    @{ Name = 'authorization header'; Pattern = 'Authorization:\s*(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{12,}' },
    @{ Name = 'registry token'; Pattern = '_authToken\s*=|//registry\.(npmjs|npmmirror)\.(org|com)/:_authToken|always-auth\s*=\s*true' },
    @{ Name = 'credential assignment'; Pattern = '(api[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|private[_-]?key|password|secret[_-]?key)\s*[:=]\s*["''](?!(?:[A-Za-z]:|/|\\|\.|%|\$|\{\{|<|replace|placeholder|example|test|dummy|fake|your))[^"'']{16,}["'']' },
    @{ Name = 'database URL with credentials'; Pattern = '(postgres|postgresql|mysql|mariadb|mongodb|redis)://[^\s:/@]+:[^\s@/]+@' }
)

$problems = [System.Collections.Generic.List[string]]::new()
$scannerPath = 'scripts/check-public-repo.ps1'

function Get-ForbiddenPathRule([string]$Path) {
    foreach ($rule in $pathRules) {
        if ([regex]::IsMatch($Path, $rule.Pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
            return $rule.Name
        }
    }
    return $null
}

Push-Location $repoRoot
try {
    $candidatePaths = @(git ls-files --cached --others --exclude-standard)
    foreach ($path in $candidatePaths) {
        $ruleName = Get-ForbiddenPathRule $path
        if ($ruleName) {
            $problems.Add("[$ruleName] $path")
        }
    }

    $textExtensions = @(
        '.c', '.conf', '.css', '.h', '.html', '.ini', '.json', '.lock', '.md', '.mjs',
        '.cjs', '.js', '.ps1', '.rs', '.sql', '.svg', '.toml', '.ts', '.tsx', '.txt',
        '.xml', '.yaml', '.yml'
    )
    $textBasenames = @('.gitignore', '.gitattributes', 'LICENSE', 'Makefile')

    foreach ($path in $candidatePaths) {
        if ((Get-ForbiddenPathRule $path) -or $path -eq $scannerPath) {
            continue
        }
        $fullPath = Join-Path $repoRoot $path
        if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
            continue
        }
        $item = Get-Item -LiteralPath $fullPath
        if ($item.Length -gt 2MB) {
            continue
        }
        $extension = [IO.Path]::GetExtension($path)
        if (($textExtensions -notcontains $extension) -and ($textBasenames -notcontains $item.Name)) {
            continue
        }
        $text = [IO.File]::ReadAllText($fullPath)
        foreach ($rule in $contentRules) {
            if ([regex]::IsMatch($text, $rule.Pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
                $problems.Add("[$($rule.Name)] $path")
            }
        }
    }

    if ($History) {
        $historyPaths = @(
            git log --all --pretty=format: --name-only |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                Sort-Object -Unique
        )
        foreach ($path in $historyPaths) {
            $ruleName = Get-ForbiddenPathRule $path
            if ($ruleName) {
                $problems.Add("[$ruleName in history] $path")
            }
        }

        $commits = @(git rev-list --all)
        foreach ($rule in $contentRules) {
            $matches = @(git grep -I -l -i -P -- $rule.Pattern $commits 2>$null)
            foreach ($file in $matches) {
                if ($file -eq $scannerPath -or $file -like "*:$scannerPath") {
                    continue
                }
                $problems.Add("[$($rule.Name) in history] $file")
            }
        }
    }
}
finally {
    Pop-Location
}

if ($problems.Count -gt 0) {
    Write-Host 'Public repository hygiene check failed:' -ForegroundColor Red
    foreach ($problem in $problems | Sort-Object -Unique) {
        Write-Host "  $problem"
    }
    Write-Host 'Remove the matching data before committing or pushing. Never print secret values.'
    exit 1
}

$historyLabel = if ($History) { ', including Git history' } else { '' }
Write-Host ("Public repository hygiene check passed ({0} working-tree files checked{1})." -f $candidatePaths.Count, $historyLabel) -ForegroundColor Green
exit 0
