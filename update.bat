@echo off
setlocal EnableDelayedExpansion
title TagGUI-rs updater

echo.
echo ============================================
echo   TagGUI-rs updater
echo ============================================
echo.

cd /d "%~dp0"

where git >nul 2>nul
if %errorlevel% neq 0 (
    echo ERROR: Git was not found, so this folder cannot be updated
    echo        automatically. Either install Git from
    echo        https://git-scm.com/downloads and try again, or download
    echo        the latest code as a ZIP and run install.bat in it.
    goto :fail
)

echo [update] Pulling the latest code...
git pull --ff-only
if %errorlevel% neq 0 (
    echo [update] ERROR: git pull failed. If you changed files locally,
    echo          commit or stash them first, then run update.bat again.
    goto :fail
)

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul
if %errorlevel% neq 0 (
    echo [update] ERROR: cargo was not found. Run install.bat first.
    goto :fail
)

rem `rustup update` can fail benignly (e.g. a locked self-update file);
rem its result is not fatal - the build below is what matters.
echo [update] Updating the Rust toolchain if needed (failures ignored)...
rustup update stable 1>nul 2>&1

rem libclang for the local-model engine's bindings generation.
if exist "C:\Program Files\LLVM\in\libclang.dll" (
    set "LIBCLANG_PATH=C:\Program Files\LLVM\in"
)

echo [update] Rebuilding (usually much faster than the first build)...
cargo build --release -p tg-gui
if %errorlevel% neq 0 (
    echo [update] The build failed once. rustup self-updates can lock the
    echo          cargo launcher for a few seconds - waiting and retrying...
    timeout /t 5 /nobreak >nul
    cargo build --release -p tg-gui
)
if %errorlevel% neq 0 (
    echo [update] Still failing; trying the toolchain cargo directly...
    set "TOOLCHAIN_CARGO=%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
    if exist "!TOOLCHAIN_CARGO!" (
        "!TOOLCHAIN_CARGO!" build --release -p tg-gui
    )
)
if %errorlevel% neq 0 (
    echo [update] ERROR: the build failed. If this is a Rust version
    echo          problem, run `rustup update` in a NEW terminal and try
    echo          again. Otherwise please open an issue with the full
    echo          output.
    goto :fail
)

echo.
echo [done] Updated and rebuilt. Double-click taggui-rs.bat to start.
echo.
pause
exit /b 0

:fail
echo.
pause
exit /b 1
