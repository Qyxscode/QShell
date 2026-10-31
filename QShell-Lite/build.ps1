# 编译 QShell Lite（命令行 csc，无需 Visual Studio）
# 用法：  powershell -ExecutionPolicy Bypass -File .\build.ps1

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$csc = "C:\Windows\Microsoft.NET\Framework\v4.0.30319\csc.exe"
if (-not (Test-Path $csc)) {
    throw "找不到 .NET Framework 4.0 的 csc.exe: $csc"
}

$out = Join-Path $PSScriptRoot "QShellLite.exe"

$refs = @(
    "System.dll",
    "System.Core.dll",
    "System.Drawing.dll",
    "System.Windows.Forms.dll",
    "System.Web.Extensions.dll"
)

# SSH.NET 2020.0.0（最后一个支持 .NET 4.0 的版本）
$sshNet = Join-Path $PSScriptRoot "lib\Renci.SshNet.dll"
if (-not (Test-Path $sshNet)) {
    throw "找不到 lib\Renci.SshNet.dll"
}

$sources = @(
    "Program.cs",
    "HostEntry.cs",
    "VtCore.cs",
    "TerminalView.cs",
    "ISession.cs",
    "SshSession.cs",
    "SerialSession.cs",
    "HostEditor.cs",
    "InputDialog.cs",
    "MemoPanel.cs",
    "MemoEditDialog.cs",
    "SettingsDialog.cs",
    "AppSettingsDialog.cs",
    "LiteForm.cs"
)

$cscArgs = @("/nologo", "/target:winexe", "/platform:x86", "/optimize+", "/out:$out")
foreach ($r in $refs) { $cscArgs += "/r:$r" }
$cscArgs += "/r:$sshNet"
$compression = Join-Path $PSScriptRoot "System.IO.Compression.dll"
if (Test-Path $compression) { $cscArgs += "/r:$compression" }
else { $cscArgs += "/r:System.IO.Compression.dll" }
foreach ($s in $sources) { $cscArgs += (Join-Path $PSScriptRoot $s) }

& $csc @cscArgs
if ($LASTEXITCODE -ne 0) {
    throw "编译失败（exit $LASTEXITCODE）"
}

# 运行时 DLL 必须和 exe 同目录
Copy-Item $sshNet (Join-Path $PSScriptRoot "Renci.SshNet.dll") -Force

$size = (Get-Item $out).Length
Write-Host ("编译成功: {0}  ({1:N0} 字节)" -f $out, $size)
