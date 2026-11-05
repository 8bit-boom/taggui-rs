@echo off
if not exist "%~dp0target\release\tag-gui.exe" (
    echo TagGUI-rs is not built yet. Run install.bat first.
    pause
    exit /b 1
)
start "" "%~dp0target\release\tag-gui.exe"
