# QShell FTP mock server (PowerShell, no dependencies)
# Usage:
#   powershell -ExecutionPolicy Bypass -File ftp-mock.ps1
# Then connect QShell FTP session to 127.0.0.1:2121 (any user/password)

param([int]$Port = 2121)

$ErrorActionPreference = "Stop"

# In-memory virtual filesystem
$remoteFiles = @{
    "readme.txt" = "QShell FTP mock server`r`nThis is a test file.`r`n"
    "hello.txt"  = "hello from ftp mock`r`n"
}
$remoteDirs = New-Object System.Collections.Generic.List[string]
$remoteDirs.Add("subdir")

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
$listener.Start()
Write-Host "========================================"
Write-Host " QShell FTP mock server started"
Write-Host (" Address: 127.0.0.1:" + $Port)
Write-Host " user: any   password: any"
Write-Host " Ctrl+C to stop"
Write-Host "========================================"

function Send-Line($stream, [string]$line) {
    $bytes = [System.Text.Encoding]::ASCII.GetBytes($line + "`r`n")
    $stream.Write($bytes, 0, $bytes.Length)
}

function Build-Listing {
    $lines = @()
    foreach ($k in ($remoteFiles.Keys | Sort-Object)) {
        $size = $remoteFiles[$k].Length
        $lines += ("-rw-r--r--    1 ftp      ftp " + $size.ToString().PadLeft(10) + " Jan 01 12:00 " + $k)
    }
    foreach ($d in $remoteDirs) {
        $lines += ("drwxr-xr-x    2 ftp      ftp       4096 Jan 01 12:00 " + $d)
    }
    return $lines
}

while ($true) {
    $client = $listener.AcceptTcpClient()
    $stream = $client.GetStream()
    $ascii = [System.Text.Encoding]::ASCII
    $reader = New-Object System.IO.StreamReader($stream, $ascii)
    Write-Host ("[connect] " + $client.Client.RemoteEndPoint)
    Send-Line $stream "220 QShell FTP mock ready"

    $dataListener = $null
    $renameFrom = $null

    while ($true) {
        $line = $null
        try { $line = $reader.ReadLine() } catch { break }
        if ($null -eq $line) { break }
        Write-Host ("[cmd] " + $line)
        $parts = $line.Split(" ", 2)
        $cmd = $parts[0].ToUpper()
        $arg = if ($parts.Length -gt 1) { $parts[1] } else { "" }

        switch ($cmd) {
            "USER" { Send-Line $stream "331 Password required" }
            "PASS" { Send-Line $stream "230 Logged in" }
            "TYPE" { Send-Line $stream "200 Type set" }
            "PWD"  { Send-Line $stream "257 ""/"" is current directory" }
            "CWD"  { Send-Line $stream "250 Directory changed" }
            "PASV" {
                if ($null -ne $dataListener) { $dataListener.Stop() }
                $dataListener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
                $dataListener.Start()
                $p = $dataListener.LocalEndpoint.Port
                $p1 = [math]::Floor($p / 256)
                $p2 = $p % 256
                Send-Line $stream ("227 Entering Passive Mode (127,0,0,1," + $p1 + "," + $p2 + ")")
            }
            "LIST" {
                Send-Line $stream "150 Opening data connection"
                try {
                    $dataClient = $dataListener.AcceptTcpClient()
                    $dataStream = $dataClient.GetStream()
                    $listing = Build-Listing
                    $payload = if ($listing.Count -gt 0) { ($listing -join "`r`n") + "`r`n" } else { "" }
                    $bytes = $ascii.GetBytes($payload)
                    $dataStream.Write($bytes, 0, $bytes.Length)
                    $dataStream.Close()
                    $dataClient.Close()
                } catch { Write-Host ("[list error] " + $_.Exception.Message) }
                Send-Line $stream "226 Transfer complete"
            }
            "RETR" {
                $name = $arg.Split("/")[-1]
                if ($remoteFiles.ContainsKey($name)) {
                    Send-Line $stream "150 Opening data connection"
                    try {
                        $dataClient = $dataListener.AcceptTcpClient()
                        $dataStream = $dataClient.GetStream()
                        $bytes = [System.Text.Encoding]::UTF8.GetBytes($remoteFiles[$name])
                        $dataStream.Write($bytes, 0, $bytes.Length)
                        $dataStream.Close()
                        $dataClient.Close()
                    } catch { Write-Host ("[retr error] " + $_.Exception.Message) }
                    Send-Line $stream "226 Transfer complete"
                } else {
                    Send-Line $stream "550 File not found"
                }
            }
            "STOR" {
                $name = $arg.Split("/")[-1]
                Send-Line $stream "150 Opening data connection"
                try {
                    $dataClient = $dataListener.AcceptTcpClient()
                    $dataStream = $dataClient.GetStream()
                    $ms = New-Object System.IO.MemoryStream
                    $buf = New-Object byte[] 8192
                    while (($n = $dataStream.Read($buf, 0, $buf.Length)) -gt 0) {
                        $ms.Write($buf, 0, $n)
                    }
                    $remoteFiles[$name] = [System.Text.Encoding]::UTF8.GetString($ms.ToArray())
                    Write-Host ("[stored] " + $name + " (" + $ms.Length + " bytes)")
                    $dataStream.Close()
                    $dataClient.Close()
                } catch { Write-Host ("[stor error] " + $_.Exception.Message) }
                Send-Line $stream "226 Transfer complete"
            }
            "DELE" {
                $name = $arg.Split("/")[-1]
                if ($remoteFiles.ContainsKey($name)) {
                    $remoteFiles.Remove($name)
                    Send-Line $stream "250 File deleted"
                } else {
                    Send-Line $stream "550 File not found"
                }
            }
            "MKD" {
                $name = $arg.Split("/")[-1]
                if (-not $remoteDirs.Contains($name)) { $remoteDirs.Add($name) }
                Send-Line $stream ("257 """ + $arg + """ created")
            }
            "RMD" {
                $name = $arg.Split("/")[-1]
                if ($remoteDirs.Contains($name)) { $remoteDirs.Remove($name) }
                Send-Line $stream "250 Directory removed"
            }
            "RNFR" {
                $renameFrom = $arg.Split("/")[-1]
                Send-Line $stream "350 Ready for RNTO"
            }
            "RNTO" {
                $to = $arg.Split("/")[-1]
                if ($renameFrom -and $remoteFiles.ContainsKey($renameFrom)) {
                    $remoteFiles[$to] = $remoteFiles[$renameFrom]
                    $remoteFiles.Remove($renameFrom)
                    Send-Line $stream "250 Renamed"
                } elseif ($renameFrom -and $remoteDirs.Contains($renameFrom)) {
                    $remoteDirs.Remove($renameFrom)
                    $remoteDirs.Add($to)
                    Send-Line $stream "250 Renamed"
                } else {
                    Send-Line $stream "550 Rename failed"
                }
                $renameFrom = $null
            }
            "QUIT" {
                Send-Line $stream "221 Bye"
                break
            }
            default {
                Write-Host ("[unknown] raw=[" + $line + "] cmd=[" + $cmd + "]")
                Send-Line $stream "502 Command not implemented"
            }
        }
    }
    $client.Close()
    Write-Host "[disconnect]"
}
