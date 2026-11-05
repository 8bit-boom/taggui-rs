@echo off
setlocal EnableDelayedExpansion
title TagGUI-rs installer

echo.
echo ============================================
echo   TagGUI-rs installer
echo ============================================
echo.
echo This installs everything needed to build and run TagGUI-rs:
echo   1. Git (optional, needed for updates)
echo   2. The Rust toolchain (rustup + cargo)
echo   3. CMake and LLVM/libclang (needed to build the local-model engine)
echo   4. Microsoft C++ build tools (only if the build needs them)
echo   5. TagGUI-rs itself (release build)
echo The first build downloads dependencies and can take 5-15 minutes.
echo.

cd /d "%~dp0"

rem ---------------------------------------------------------------------------
rem 1. Git (only needed for update.bat; a ZIP download works without it).
rem ---------------------------------------------------------------------------
where git >nul 2>nul
if %errorlevel% neq 0 (
    echo [git] Git was not found. It is only needed for automatic updates
    echo        ^(update.bat^). You can install it later from
    echo        https://git-scm.com/downloads - continuing without it.
) else (
    echo [git] Found.
)

rem ---------------------------------------------------------------------------
rem 2. Rust toolchain.
rem ---------------------------------------------------------------------------
set "CARGO_BIN=%USERPROFILE%\.cargo\bin"
set "PATH=%CARGO_BIN%;%PATH%"

where cargo >nul 2>nul
if %errorlevel% equ 0 (
    echo [rust] Found.
    goto :build
)

echo [rust] Rust was not found. Installing the Rust toolchain...
where winget >nul 2>nul
if %errorlevel% equ 0 (
    echo [rust] Installing rustup via winget...
    winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
) else (
    echo [rust] winget not available; downloading rustup-init.exe...
    curl -L --fail -o "%TEMP%\rustup-init.exe" https://win.rustup.rs/x86_64
    if !errorlevel! neq 0 (
        echo [rust] ERROR: could not download rustup-init.exe.
        echo        Download it manually from https://rustup.rs and run:
        echo            rustup-init.exe -y
        goto :fail
    )
    "%TEMP%\rustup-init.exe" -y --default-toolchain stable-x86_64-pc-windows-msvc
)

rem rustup installs into %USERPROFILE%\.cargo\bin - pick it up in this
rem session without restarting the terminal.
if exist "%CARGO_BIN%\cargo.exe" (
    echo [rust] Installed.
) else (
    echo [rust] ERROR: cargo was not installed. Open a NEW terminal window
    echo        and run install.bat again. If that fails, install Rust
    echo        manually from https://rustup.rs
    goto :fail
)

:build
rem ---------------------------------------------------------------------------
rem 2b. CMake and LLVM/libclang - the local GGUF captioning engine
rem     (llama.cpp) is built from C sources at compile time and its
rem     bindings are generated with libclang.
rem ---------------------------------------------------------------------------
where cmake >nul 2>nul
if %errorlevel% neq 0 (
    echo [cmake] CMake was not found. Installing via winget...
    where winget >nul 2>nul
    if %errorlevel% equ 0 (
        winget install --id Kitware.CMake -e --accept-source-agreements --accept-package-agreements
        echo [cmake] If the build below still cannot find CMake, open a NEW
        echo        terminal and run install.bat again.
    ) else (
        echo [cmake] WARNING: winget not available - install CMake manually
        echo        from https://cmake.org/download/ and re-run install.bat.
    )
) else (
    echo [cmake] Found.
)
set "LIBCLANG_PATH=%LIBCLANG_PATH%"
if exist "C:\Program Files\LLVM\bin\libclang.dll" (
    echo [llvm] Found libclang.
    set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
) else (
    echo [llvm] LLVM/libclang not found. Installing via winget...
    where winget >nul 2>nul
    if %errorlevel% equ 0 (
        winget install --id LLVM.LLVM -e --accept-source-agreements --accept-package-agreements
        if exist "C:\Program Files\LLVM\bin\libclang.dll" (
            set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
            rem Persist for future terminals too.
            setx LIBCLANG_PATH "C:\Program Files\LLVM\bin" >nul
        ) else (
            echo [llvm] WARNING: libclang still missing - the local-model
            echo        engine will not build. Install LLVM from
            echo        https://releases.llvm.org and re-run install.bat.
        )
    ) else (
        echo [llvm] WARNING: winget not available - install LLVM manually
        echo        from https://releases.llvm.org and set LIBCLANG_PATH.
    )
)

rem ---------------------------------------------------------------------------
rem 3. Build TagGUI-rs.
rem ---------------------------------------------------------------------------
echo.
echo [build] Building TagGUI-rs (release)...
cargo build --release -p tg-gui
if %errorlevel% neq 0 (
    echo [build] The build failed once. A just-updated rustup can lock its
    echo          launcher for a few seconds - waiting and retrying...
    timeout /t 5 /nobreak >nul
    cargo build --release -p tg-gui
)
if %errorlevel% equ 0 goto :build_ok

echo.
echo [build] The build failed. On a fresh Windows machine this is almost
echo         always a missing Microsoft C++ linker (MSVC). Installing the
echo         Visual Studio Build Tools now (about 2-7 GB; this takes a
echo         while)...
where winget >nul 2>nul
if %errorlevel% equ 0 (
    winget install --id Microsoft.VisualStudio.2022.BuildTools -e ^
        --accept-source-agreements --accept-package-agreements ^
        --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
) else (
    echo [build] winget not available; downloading the Visual Studio Build
    echo         Tools installer directly...
    curl -L --fail -o "%TEMP%\vs_BuildTools.exe" https://aka.ms/vs/17/release/vs_BuildTools.exe
    if !errorlevel! neq 0 (
        echo [build] ERROR: could not download the Build Tools installer.
        goto :fail
    )
    "%TEMP%\vs_BuildTools.exe" --quiet --wait --norestart ^
        --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended
)

echo.
echo [build] Retrying the build (a NEW terminal window may be needed if
echo         this still fails - reopen one and run install.bat again)...
cargo build --release -p tg-gui
if %errorlevel% neq 0 (
    echo [build] ERROR: the build still fails. Try these, in order:
    echo         1. Close this window, open a NEW terminal, run install.bat
    echo            again (the build tools installation above may not be
    echo            visible to this session yet).
    echo         2. rustup update
    echo         3. If nothing helps, open an issue with the full output.
    goto :fail
)

:build_ok
echo.
echo [done] Build succeeded.
if not exist "target\release\tag-gui.exe" (
    echo [done] ERROR: expected binary is missing - target\release\tag-gui.exe
    goto :fail
)

rem ---------------------------------------------------------------------------
rem 4. Launcher.
rem ---------------------------------------------------------------------------
echo [done] Double-click taggui-rs.bat to start the app.
echo        Use update.bat to pull new versions and rebuild.
echo.
choice /C YN /M "Launch TagGUI-rs now"
if !errorlevel! equ 1 start "" "%~dp0target\release\tag-gui.exe"
exit /b 0

:fail
echo.
echo Installation did not complete. Fix the issue above and run
echo install.bat again - it is safe to re-run at any time.
echo.
pause
exit /b 1
