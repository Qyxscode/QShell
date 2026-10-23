param()

$ErrorActionPreference = "Stop"

$invokedPath = $MyInvocation.MyCommand.Path
if ($invokedPath) {
    $scriptDir = Split-Path -Parent $invokedPath
} elseif ($env:CARGO_MANIFEST_DIR) {
    $scriptDir = $env:CARGO_MANIFEST_DIR
} else {
    $scriptDir = (Get-Location).Path
}

$projectRoot = $scriptDir
$qshellNextDir = Join-Path (Split-Path -Parent $projectRoot) "qshell-next"
$uiDistDir = Join-Path $projectRoot "ui_dist"

Write-Host "=== [QShell Sync] 正在同步前端构建产物 ==="
Write-Host "执行上下文目录: $((Get-Location).Path)"
Write-Host "脚本识别项目根目录: $projectRoot"
Write-Host "前端项目路径: $qshellNextDir"
Write-Host "目标产物路径: $uiDistDir"

if (Test-Path $qshellNextDir) {
    Push-Location $qshellNextDir
    try {
        Write-Host "执行前端编译 (npm run build)..."
        npm run build
        if ($LASTEXITCODE -ne 0) {
            throw "npm run build 失败，退出码: $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }

    $distDir = Join-Path $qshellNextDir "dist"
    if (Test-Path $distDir) {
        if (-not (Test-Path (Join-Path $distDir "index.html"))) {
            throw "前端构建产物缺少 index.html: $distDir"
        }
        if (Test-Path $uiDistDir) {
            Write-Host "清理旧静态资源目录..."
            Remove-Item -Path $uiDistDir -Recurse -Force
        }
        New-Item -ItemType Directory -Path $uiDistDir -Force | Out-Null
        Write-Host "复制最新编译产物到 ui_dist..."
        Copy-Item -Path (Join-Path $distDir "*") -Destination $uiDistDir -Recurse -Force
        Write-Host "=== [QShell Sync] 前端资源同步完成 ==="
    } else {
        throw "未找到前端构建输出目录: $distDir"
    }
} else {
    throw "未找到 qshell-next 前端源码目录: $qshellNextDir"
}
