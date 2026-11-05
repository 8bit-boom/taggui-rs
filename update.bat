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
if exist "C:\Program Files\LLVM\bin\libclang.dll" (
    set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
)

rem Rebuild with the same GPU features install.bat chose: CUDA when an
rem NVIDIA GPU + toolkit are present, Vulkan when the SDK is installed.
set "BUILD_FEATURES="
call :find_nvcc
if defined NVCC_PATH (
    set "BUILD_FEATURES=--features tg-caption/cuda"
    goto :gpu_found
)
for /d %%D in ("C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v*") do (
    if exist "%%D\bin\nvcc.exe" (
        set "CUDA_DIR=%%D"
        set "BUILD_FEATURES=--features tg-caption/cuda"
    )
)
:gpu_found
if defined BUILD_FEATURES (
    echo [update] GPU build: CUDA toolkit found, building with CUDA.
) else (
    echo [update] No CUDA toolkit found - building CPU-only. ^(Run
    echo          install.bat on a machine with an NVIDIA GPU to get a
    echo          CUDA build.^)
)

rem MSBuild's CUDA build integration resolves the toolkit directory from
rem the version-specific CUDA_PATH_Vx_y variable (or the registry
rem InstallDir value) - NOT the generic CUDA_PATH. A session started
rem before the toolkit was installed has neither, so wire them up here
rem from wherever nvcc was found.
if defined CUDA_DIR (
    set "CUDA_PATH=!CUDA_DIR!"
    for %%V in ("!CUDA_DIR!") do set "CUDA_FOLDER=%%~nxV"
    set "CUDA_VER=!CUDA_FOLDER:v=V!"
    set "CUDA_VER=!CUDA_VER:.=_!"
    set "CUDA_PATH_!CUDA_VER!=!CUDA_DIR!"
    set "PATH=!CUDA_DIR!\bin;%PATH%"
    echo [update] CUDA env wired: CUDA_PATH and CUDA_PATH_!CUDA_VER!
    rem Some winget CUDA installs leave the registry InstallDir value out,
    rem which makes MSBuild's CUDA integration resolve CudaToolkitDir to
    rem empty. Repair it when missing (best-effort; needs admin - the
    rem version-specific env var above covers the non-admin case).
    reg query "HKLM\SOFTWARE\NVIDIA Corporation\GPU Computing Toolkit\CUDA\!CUDA_FOLDER!" /v InstallDir >nul 2>nul
    if !errorlevel! neq 0 (
        reg add "HKLM\SOFTWARE\NVIDIA Corporation\GPU Computing Toolkit\CUDA\!CUDA_FOLDER!" /v InstallDir /t REG_SZ /d "!CUDA_DIR!" /f >nul 2>&1
        echo [update] Repaired the CUDA registry InstallDir entry.
    )
)

rem A previously interrupted GPU build can leave a partial CMake cache
rem ^(CMakeCache.txt without the generated project^); the cmake crate then
rem skips reconfiguration and every build dies with MSB1009. Remove any
rem such broken build tree before building.
call :purge_broken_cmake

echo [update] Rebuilding (usually much faster than the first build)...
cargo build --release -p tg-gui !BUILD_FEATURES!
if %errorlevel% neq 0 (
    echo [update] The build failed once. rustup self-updates can lock the
    echo          cargo launcher for a few seconds - waiting and retrying...
    timeout /t 5 /nobreak >nul
    cargo build --release -p tg-gui !BUILD_FEATURES!
)
if %errorlevel% neq 0 (
    echo [update] Still failing; clearing the llama.cpp cmake build trees
    echo          and retrying ^(a partial cache from an interrupted build
    echo          is the usual cause; the retry recompiles from scratch^)...
    call :purge_all_cmake
    cargo build --release -p tg-gui !BUILD_FEATURES!
)
if %errorlevel% neq 0 (
    echo [update] One more retry, CPU-only ^(the GPU feature may be the
    echo          problem^)...
    cargo build --release -p tg-gui
)
if %errorlevel% neq 0 (
    echo [update] ERROR: the build failed. If this is a Rust version
    echo          problem, run `rustup update` in a NEW terminal and try
    echo          again. Otherwise please open an issue with the full
    echo          output.
    goto :fail
)

rem Copy the CUDA runtime DLLs next to the exe (same as install.bat).
if defined CUDA_DIR (
    call :copy_cuda_dlls "!CUDA_DIR!"
)

echo.
echo [done] Updated and rebuilt. Double-click start.bat to start.
echo.
pause
exit /b 0

:find_nvcc
rem Locate nvcc: PATH first, then the standard CUDA install directories.
rem Sets NVCC_PATH and CUDA_DIR (the toolkit root).
set "NVCC_PATH="
set "CUDA_DIR="
where nvcc >nul 2>nul
if %errorlevel% equ 0 (
    for /f "delims=" %%P in ('where nvcc') do (
        set "NVCC_PATH=%%P"
        for %%D in ("%%P\..\..") do set "CUDA_DIR=%%~fD"
        goto :find_nvcc_done
    )
)
for /d %%D in ("C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v*") do (
    if exist "%%D\bin\nvcc.exe" (
        set "NVCC_PATH=%%D\bin\nvcc.exe"
        set "CUDA_DIR=%%D"
    )
)
:find_nvcc_done
exit /b 0

:purge_broken_cmake
rem Remove llama.cpp cmake build trees that were configured but never
rem finished (CMakeCache.txt present, generated project missing) - they
rem make every subsequent build fail with MSB1009.
set "PURGED="
for /d %%F in ("target\release\build\llama-cpp-sys-2-*") do (
    if exist "%%F\out\build\CMakeCache.txt" (
        if not exist "%%F\out\build\INSTALL.vcxproj" (
            echo [update] Removing a stale partial cmake build tree...
            rd /s /q "%%F\out\build"
            set "PURGED=1"
        )
    )
)
exit /b 0

:purge_all_cmake
rem Nuclear option: remove every llama.cpp cmake build tree so the next
rem build reconfigures and recompiles from scratch.
for /d %%F in ("target\release\build\llama-cpp-sys-2-*") do (
    if exist "%%F\out\build" (
        echo [update] Clearing %%F\out\build ...
        rd /s /q "%%F\out\build"
    )
)
exit /b 0

:copy_cuda_dlls
rem %1 = the CUDA toolkit root; copies the runtime DLLs the local-model
rem engine links against next to tag-gui.exe (bin for CUDA 12, bin\x64
rem for CUDA 13). Lists must come from dir /b - a plain for-glob expands
rem against the current directory.
set "CUDA_BIN=%~1\bin"
if not exist "target\release" exit /b 0
set "COPIED="
for %%S in ("%CUDA_BIN%" "%CUDA_BIN%\x64") do (
    for /f "delims=" %%F in ('dir /b "%%~S\cudart64_*.dll" 2^>nul') do (
        copy /y "%%~S\%%F" "target\release\%%F" >nul
        set "COPIED=1"
    )
    for /f "delims=" %%F in ('dir /b "%%~S\cublas64_*.dll" 2^>nul') do (
        copy /y "%%~S\%%F" "target\release\%%F" >nul
        set "COPIED=1"
    )
    for /f "delims=" %%F in ('dir /b "%%~S\cublasLt64_*.dll" 2^>nul') do (
        copy /y "%%~S\%%F" "target\release\%%F" >nul
        set "COPIED=1"
    )
)
if defined COPIED echo [done] CUDA runtime DLLs copied next to the app.
exit /b 0

:fail
echo.
pause
exit /b 1
