[CmdletBinding()]
param(
    [switch]$Run,
    [switch]$SkipTypeCheck,
    [switch]$Clean
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location -LiteralPath $projectRoot

$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

function Write-Step {
    param([string]$Message)
    Write-Host ""
    Write-Host ("==> " + $Message) -ForegroundColor Cyan
}

function Fail {
    param([string]$Message)
    Write-Host ""
    Write-Host ("[失败] " + $Message) -ForegroundColor Red
    exit 1
}

Write-Host "============================================" -ForegroundColor DarkGray
Write-Host "  QShell 一键打包 (单文件 exe)" -ForegroundColor White
Write-Host ("  项目目录: " + $projectRoot) -ForegroundColor DarkGray
Write-Host "============================================" -ForegroundColor DarkGray

# ---------- 定位 Node.js ----------
$node = $null
$nodeCmd = Get-Command node -ErrorAction SilentlyContinue
if ($nodeCmd) { $node = $nodeCmd.Source }

if (-not $node) {
    $nodeCandidates = @(
        (Join-Path $env:ProgramFiles "nodejs\node.exe"),
        (Join-Path ${env:ProgramFiles(x86)} "nodejs\node.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\nodejs\node.exe"),
    )
    foreach ($candidate in $nodeCandidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) {
            $node = $candidate
            break
        }
    }
}
if (-not $node) {
    Fail "未找到 Node.js。请先安装 Node.js，或把 node.exe 加入 PATH。"
}
Write-Host ("Node:   " + $node) -ForegroundColor DarkGray

# ---------- 定位 Cargo ----------
$cargo = $null
$cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCmd) { $cargo = $cargoCmd.Source }

if (-not $cargo) {
    $cargoCandidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path -LiteralPath $cargoCandidate) { $cargo = $cargoCandidate }
}
if (-not $cargo) {
    Fail "未找到 cargo。请先安装 Rust 工具链，或把 %USERPROFILE%\.cargo\bin 加入 PATH。"
}
$env:PATH = (Split-Path -Parent $cargo) + ";" + $env:PATH
Write-Host ("Cargo:  " + $cargo) -ForegroundColor DarkGray

# ---------- 检查前端依赖 ----------
$viteCli = Join-Path $projectRoot "node_modules\vite\bin\vite.js"
$vueTscCli = Join-Path $projectRoot "node_modules\vue-tsc\bin\vue-tsc.js"
if (-not (Test-Path -LiteralPath $viteCli)) {
    Fail "缺少 node_modules。请先在项目目录执行 npm install。"
}

# ---------- 读取版本号与 crate 名 ----------
$packageJson = Get-Content -LiteralPath (Join-Path $projectRoot "package.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$tauriConf = Get-Content -LiteralPath (Join-Path $projectRoot "src-tauri\tauri.conf.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$cargoToml = Get-Content -LiteralPath (Join-Path $projectRoot "src-tauri\Cargo.toml") -Raw -Encoding UTF8

$version = $tauriConf.version
if (-not $version) { $version = $packageJson.version }

if ($packageJson.version -ne $version) {
    Write-Host ("[警告] package.json 版本 " + $packageJson.version + " 与 tauri.conf.json 版本 " + $version + " 不一致，输出文件名以 tauri.conf.json 为准。") -ForegroundColor Yellow
}

if ($cargoToml -match '(?m)^name\s*=\s*"([^"]+)"') {
    $crateName = $matches[1]
} else {
    Fail "无法从 src-tauri\Cargo.toml 读取 package.name。"
}
Write-Host ("版本:   " + $version + "     crate: " + $crateName) -ForegroundColor DarkGray

$builtExe = Join-Path $projectRoot ("src-tauri\target\release\" + $crateName + ".exe")
$outputExe = Join-Path $projectRoot ("QShell_" + $version + ".exe")

# ---------- 检查目标 exe 是否正在运行（避免覆盖失败） ----------
$runningNames = @($crateName, [System.IO.Path]::GetFileNameWithoutExtension($outputExe))
foreach ($name in $runningNames) {
    $running = Get-Process -Name $name -ErrorAction SilentlyContinue
    foreach ($proc in @($running)) {
        if (-not $proc) { continue }
        $procPath = $null
        try { $procPath = $proc.Path } catch { }
        if (-not $procPath) { continue }
        foreach ($target in @($builtExe, $outputExe)) {
            try {
                $same = [string]::Equals(
                    [System.IO.Path]::GetFullPath($procPath),
                    [System.IO.Path]::GetFullPath($target),
                    [System.StringComparison]::OrdinalIgnoreCase
                )
                if ($same) {
                    Fail ("检测到目标程序正在运行 (" + $procPath + ")，请先关闭该实例再重新打包。")
                }
            } catch { }
        }
    }
}

# ---------- 可选清理 ----------
if ($Clean) {
    Write-Step "清理 " + $crateName + " 的编译产物"
    Push-Location -LiteralPath (Join-Path $projectRoot "src-tauri")
    try {
        & $cargo clean -p $crateName
        if ($LASTEXITCODE -ne 0) { Fail "cargo clean 失败" }
    } finally {
        Pop-Location
    }
}

# ---------- 1. 类型检查 ----------
if (-not $SkipTypeCheck) {
    Write-Step "1/3 TypeScript 类型检查"
    & $node $vueTscCli --noEmit
    if ($LASTEXITCODE -ne 0) { Fail "TypeScript 类型检查未通过" }
} else {
    Write-Host ""
    Write-Host "==> 1/3 已跳过 TypeScript 类型检查" -ForegroundColor DarkGray
}

# ---------- 2. 前端构建 ----------
Write-Step "2/3 构建前端 (vite build -> dist)"
& $node $viteCli build
if ($LASTEXITCODE -ne 0) { Fail "前端构建失败" }

# ---------- 3. Rust 生产编译 ----------
Write-Step "3/3 编译生产版 exe (cargo release + custom-protocol)"
Push-Location -LiteralPath (Join-Path $projectRoot "src-tauri")
try {
    & $cargo build --release --features custom-protocol
    if ($LASTEXITCODE -ne 0) { Fail "Rust 编译失败" }
} finally {
    Pop-Location
}

# ---------- 复制并输出 ----------
if (-not (Test-Path -LiteralPath $builtExe)) {
    Fail ("未找到编译产物: " + $builtExe)
}

Copy-Item -LiteralPath $builtExe -Destination $outputExe -Force

$file = Get-Item -LiteralPath $outputExe
$hash = (Get-FileHash -LiteralPath $outputExe -Algorithm SHA256).Hash
$stopwatch.Stop()

Write-Host ""
Write-Host "============================================" -ForegroundColor Green
Write-Host "  打包完成" -ForegroundColor Green
Write-Host "============================================" -ForegroundColor Green
Write-Host ("输出文件: " + $outputExe) -ForegroundColor White
Write-Host ("文件大小: " + ("{0:N0}" -f $file.Length) + " 字节")
Write-Host ("SHA256:  " + $hash)
Write-Host ("耗时:    " + [Math]::Round($stopwatch.Elapsed.TotalSeconds, 1) + " 秒")
Write-Host ""

if ($Run) {
    Write-Host "正在启动新构建的程序..." -ForegroundColor Cyan
    Start-Process -FilePath $outputExe -WorkingDirectory $projectRoot
}

exit 0
