@echo off
setlocal
set "EXE=%~dp0target\release\QShell.exe"
if not exist "%EXE%" (
  echo 未找到 %EXE%
  pause
  exit /b 1
)
echo ============================================
echo   QShell 启动诊断
echo ============================================
echo.
echo 1) 马上会弹出“无法验证发布者”安全警告，请点【运行(R)】。
echo 2) 之后本窗口每 2 秒检查一次进程，共 10 次。
echo.
pause
echo.
echo 正在通过 shell 启动（模拟双击）...
start "" "%EXE%"
echo.
set FOUND=0
for /l %%i in (1,1,10) do (
  timeout /t 2 /nobreak >nul
  tasklist /fi "imagename eq QShell.exe" 2>nul | find /i "QShell.exe" >nul
  if errorlevel 1 (
    echo [%%i] 未发现 QShell.exe 进程
  ) else (
    echo [%%i] 已发现 QShell.exe 进程
    set FOUND=1
  )
)
echo.
if "%FOUND%"=="1" (
  echo 结果：进程已启动。如果看不到窗口，请告诉我任务栏里有没有它。
) else (
  echo 结果：20 秒内没有任何 QShell.exe 进程，说明“运行”没有真正放行。
)
echo.
echo 按任意键退出本诊断窗口（不会关闭 QShell）。
pause >nul
endlocal