@echo off
setlocal
chcp 65001 >nul
cd /d "%~dp0"

echo ============================================
echo   QShell One-Click Build
echo ============================================
echo.

powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" %*
set "RC=%ERRORLEVEL%"

echo.
if not "%RC%"=="0" (
    echo [FAILED] exit code %RC%
) else (
    echo [OK] build finished
)
echo.
pause
exit /b %RC%
