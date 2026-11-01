@echo off
setlocal
cd /d "%~dp0app\src-tauri"

echo Baue H-Mouse Config (schnell) ...
cargo build --profile fast
if errorlevel 1 goto fehler

if not exist "%~dp0release" mkdir "%~dp0release"
copy /y "target\fast\h-mouse-config.exe" "%~dp0release\h-mouse-config.exe" >nul
if errorlevel 1 goto fehler

echo.
echo Fertig: %~dp0release\h-mouse-config.exe
pause
exit /b 0

:fehler
echo.
echo Fehler beim Erstellen oder Kopieren. Falls H-Mouse Config noch laeuft, bitte schliessen.
pause
exit /b 1
