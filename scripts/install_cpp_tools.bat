@echo off
echo ========================================================
echo Installing C++ Desktop Development Tools for Visual Studio
echo (Provides link.exe and Windows SDK required for Rust builds)
echo ========================================================
echo.

set INSTALLER_PATH="C:\Program Files (x86)\Microsoft Visual Studio\Installer\setup.exe"
set VS_PATH="C:\Program Files\Microsoft Visual Studio\18\Community"

if not exist %INSTALLER_PATH% (
    echo [ERROR] Visual Studio Installer not found at %INSTALLER_PATH%.
    echo Please open Visual Studio Installer from the Start Menu.
    pause
    exit /b 1
)

echo Launching Visual Studio Installer to add Desktop C++ tools...
%INSTALLER_PATH% modify --installPath %VS_PATH% --add Microsoft.VisualStudio.Workload.NativeDesktop --includeRecommended --passive

echo.
echo ========================================================
echo Installation finished. You can close this window.
echo ========================================================
pause
