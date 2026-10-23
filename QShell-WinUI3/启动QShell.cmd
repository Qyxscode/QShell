@echo off
set "EXE=%~dp0target\release\QShell.exe"
if not exist "%EXE%" (
  echo Œ¥’“µΩ %EXE%
  pause
  exit /b 1
)
powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command "$p = New-Object System.Diagnostics.ProcessStartInfo; $p.FileName = '%EXE%'; $p.UseShellExecute = $false; [System.Diagnostics.Process]::Start($p) | Out-Null"