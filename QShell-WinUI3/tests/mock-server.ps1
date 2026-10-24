# QShell 测试用模拟服务器（无需 Python）
# 用法：
#   powershell -ExecutionPolicy Bypass -File mock-server.ps1 -Mode raw
#   powershell -ExecutionPolicy Bypass -File mock-server.ps1 -Mode rlogin

param(
    [ValidateSet("raw", "rlogin")]
    [string]$Mode = "raw",
    [int]$Port = 0
)

$ErrorActionPreference = "Stop"
if ($Port -eq 0) {
    $Port = if ($Mode -eq "raw") { 19000 } else { 1513 }
}

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
$listener.Start()
Write-Host "========================================"
Write-Host (" QShell mock " + $Mode.ToUpper() + " server started")
Write-Host (" Address: 127.0.0.1:" + $Port)
Write-Host " Press Ctrl+C to stop"
Write-Host "========================================"

function Invoke-Client([System.Net.Sockets.TcpClient]$client, [bool]$isRlogin) {
    $stream = $client.GetStream()
    $stream.ReadTimeout = 60000
    $stream.WriteTimeout = 10000
    $utf8 = [System.Text.Encoding]::UTF8

    if ($isRlogin) {
        $fields = New-Object System.Collections.Generic.List[string]
        $current = New-Object System.Collections.Generic.List[byte]
        $one = New-Object byte[] 1
        while ($fields.Count -lt 4) {
            try {
                $n = $stream.Read($one, 0, 1)
            } catch {
                $client.Close()
                return
            }
            if ($n -le 0) { $client.Close(); return }
            if ($one[0] -eq 0) {
                $fields.Add($utf8.GetString($current.ToArray()))
                $current.Clear()
            } else {
                $current.Add($one[0])
            }
        }
        Write-Host ("[rlogin] client=" + $fields[0] + " server=" + $fields[1] + " term=" + $fields[2] + " speed=" + $fields[3])
        $stream.Write([byte[]]@(0), 0, 1)
        $welcome = "`r`n=== Rlogin mock server ===`r`n" +
                   ("user: " + $fields[1] + "   term: " + $fields[2] + "`r`n") +
                   "type something and press Enter, it will be echoed.`r`n`r`n"
    } else {
        $welcome = "=== Raw TCP echo server ===`r`n" +
                   "type something and press Enter, it will be echoed.`r`n`r`n"
    }

    $bytes = $utf8.GetBytes($welcome)
    $stream.Write($bytes, 0, $bytes.Length)

    $buffer = New-Object byte[] 4096
    while ($true) {
        $n = 0
        try {
            $n = $stream.Read($buffer, 0, $buffer.Length)
        } catch [System.IO.IOException] {
            # 读超时（60 秒无输入）继续等待，不主动断开
            continue
        } catch {
            break
        }
        if ($n -le 0) { break }
        $text = $utf8.GetString($buffer, 0, $n)
        Write-Host ("[recv] " + $text.Replace("`r", "<CR>"))
        $text = $text.Replace("`r", "`r`n")
        $out = $utf8.GetBytes($text)
        $stream.Write($out, 0, $out.Length)
    }
    $client.Close()
}

$isRlogin = ($Mode -eq "rlogin")
while ($true) {
    $client = $listener.AcceptTcpClient()
    Write-Host ("[connect] " + $client.Client.RemoteEndPoint)
    try {
        Invoke-Client -client $client -isRlogin $isRlogin
    } catch {
        Write-Host ("[error] " + $_.Exception.Message)
    }
    Write-Host "[disconnect]"
}