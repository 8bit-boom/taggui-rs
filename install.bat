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
echo   5. GPU support: CUDA (NVIDIA) or Vulkan (AMD/Intel) when a GPU is
echo      found - this is what makes local captioning fast (llama.cpp's
echo      GPU kernels replace the Python app's SageAttention/FlashAttention)
echo   6. TagGUI-rs itself, built with GPU features when possible
echo The first build downloads dependencies and can take 5-15 minutes
echo (longer with the CUDA build).
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
rem 2c. GPU detection and acceleration setup. llama.cpp's CUDA/Vulkan
rem     kernels (with built-in flash attention) are the fast path; the
rem     toolchain is installed automatically when a GPU is found.
rem ---------------------------------------------------------------------------
set "BUILD_FEATURES="
nvidia-smi --query-gpu=name --format=csv,noheader >nul 2>nul
if %errorlevel% equ 0 (
    echo [gpu] NVIDIA GPU detected:
    nvidia-smi --query-gpu=name,memory.total --format=csv,noheader
    call :find_nvcc
    if defined NVCC_PATH (
        echo [gpu] CUDA toolkit found - building with CUDA support.
        set "BUILD_FEATURES=--features tg-caption/cuda"
        goto :gpu_done
    )
    echo [gpu] CUDA toolkit not installed. It enables GPU-accelerated local
    echo        captioning ^(the fast path^). It is a large download
    echo        ^(3-4 GB^) and takes several minutes to install.
    where winget >nul 2>nul
    if %errorlevel% equ 0 (
        choice /C YN /M "Install the CUDA toolkit now"
        if !errorlevel! equ 1 (
            echo [gpu] Installing the CUDA toolkit via winget...
            winget install --id Nvidia.CUDA -e --accept-source-agreements --accept-package-agreements
            call :find_nvcc
            if defined NVCC_PATH (
                rem CUDA_PATH/session wiring happened in :find_nvcc.
                setx CUDA_PATH "!CUDA_DIR!" >nul
                set "BUILD_FEATURES=--features tg-caption/cuda"
                echo [gpu] CUDA installed - building with CUDA support.
                goto :gpu_done
            )
            echo [gpu] CUDA was installed but nvcc is not visible to this
            echo        session yet. Open a NEW terminal and run install.bat
            echo        again to get a CUDA build; continuing CPU-only.
        ) else (
            echo [gpu] Skipping CUDA - building CPU-only.
        )
    ) else (
        echo [gpu] winget not available - install the CUDA toolkit from
        echo        https://developer.nvidia.com/cuda-downloads and re-run
        echo        install.bat for a GPU build. Continuing CPU-only.
    )
    goto :gpu_done
)

rem No NVIDIA GPU: check for other display adapters (AMD/Intel) and offer
rem Vulkan - llama.cpp's other GPU backend.
powershell -NoProfile -Command "if (Get-CimInstance Win32_VideoController | Where-Object { $_.Name -notmatch 'NVIDIA|Basic|Hyper-V' }) { exit 0 } else { exit 1 }" >nul 2>nul
if %errorlevel% equ 0 (
    echo [gpu] Non-NVIDIA GPU detected ^(AMD/Intel^). Vulkan gives GPU
    echo        acceleration for local captioning on these cards.
    where winget >nul 2>nul
    if %errorlevel% equ 0 (
        choice /C YN /M "Install the Vulkan SDK now"
        if !errorlevel! equ 1 (
            echo [gpu] Installing the Vulkan SDK via winget...
            winget install --id KhronosGroup.VulkanSDK -e --accept-source-agreements --accept-package-agreements
            set "BUILD_FEATURES=--features tg-caption/vulkan"
            echo [gpu] Vulkan SDK installed - building with Vulkan support.
            goto :gpu_done
        )
    ) else (
        echo [gpu] winget not available - install the Vulkan SDK from
        echo        https://vulkan.lunarg.com/sdk/home and re-run
        echo        install.bat for a GPU build.
    )
) else (
    echo [gpu] No GPU detected - local captioning will run on CPU.
    echo        The remote API backend is unaffected.
)

:gpu_done

rem A previously interrupted GPU build can leave a partial CMake cache
rem ^(CMakeCache.txt without the generated project^); the cmake crate then
rem skips reconfiguration and every build dies with MSB1009. Remove any
rem such broken build tree before building.
call :purge_broken_cmake

rem ---------------------------------------------------------------------------
rem 3. Build TagGUI-rs with the chosen features.
rem ---------------------------------------------------------------------------
echo.
echo [build] Building TagGUI-rs !BUILD_FEATURES! ...
cargo build --release -p tg-gui !BUILD_FEATURES!
if %errorlevel% neq 0 (
    echo [build] The build failed once. A just-updated rustup can lock its
    echo          launcher for a few seconds - waiting and retrying...
    timeout /t 5 /nobreak >nul
    cargo build --release -p tg-gui !BUILD_FEATURES!
)
if %errorlevel% equ 0 goto :build_ok

echo.
echo [build] The build failed. On a fresh Windows machine this is almost
echo         always a missing Microsoft C++ linker ^(MSVC^). Installing the
echo         Visual Studio Build Tools now ^(about 2-7 GB; this takes a
echo         while^)...
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
cargo build --release -p tg-gui !BUILD_FEATURES!
if %errorlevel% neq 0 (
    echo [build] The build failed again. If the GPU feature is the
    echo         problem, retrying CPU-only so you still get a working
    echo         app...
    cargo build --release -p tg-gui
    if %errorlevel% equ 0 goto :build_ok_cpu
    echo [build] ERROR: the CPU-only build also fails. Try these, in order:
    echo         1. Close this window, open a NEW terminal, run install.bat
    echo            again (installations above may not be visible yet).
    echo         2. rustup update
    echo         3. If nothing helps, open an issue with the full output.
    goto :fail
)

:build_ok_cpu
echo.
echo [done] Build succeeded (CPU-only fallback).
goto :finish

:build_ok
echo.
echo [done] Build succeeded.
:finish
if not exist "target\release\tag-gui.exe" (
    echo [done] ERROR: expected binary is missing - target\release\tag-gui.exe
    goto :fail
)

rem ---------------------------------------------------------------------------
rem 3b. Copy the CUDA runtime DLLs next to the exe (cudart/cublas), so the
rem     app runs without the toolkit on PATH - and on machines that only
rem     got the binary.
rem ---------------------------------------------------------------------------
if defined CUDA_DIR (
    call :copy_cuda_dlls "!CUDA_DIR!"
)

rem ---------------------------------------------------------------------------
rem 4. Launcher.
rem ---------------------------------------------------------------------------
echo [done] Double-click start.bat to start the app.
echo        Use update.bat to pull new versions and rebuild.
echo.
choice /C YN /M "Launch TagGUI-rs now"
if !errorlevel! equ 1 start "" "%~dp0target\release\tag-gui.exe"
exit /b 0

:find_nvcc
rem Locate nvcc: PATH first, then the standard CUDA install directories.
rem Sets NVCC_PATH and CUDA_DIR (toolkit root) and wires CUDA_PATH into
rem this session (MSBuild's CUDA targets need it - the installer's
rem machine-level value is not visible to a running parent shell).
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
if defined CUDA_DIR (
    set "CUDA_PATH=!CUDA_DIR!"
    for %%V in ("!CUDA_DIR!") do set "CUDA_FOLDER=%%~nxV"
    set "CUDA_VER=!CUDA_FOLDER:v=V!"
    set "CUDA_VER=!CUDA_VER:.=_!"
    set "CUDA_PATH_!CUDA_VER!=!CUDA_DIR!"
    set "PATH=!CUDA_DIR!\bin;%PATH%"
    rem Some winget CUDA installs leave the registry InstallDir value out,
    rem which makes MSBuild's CUDA integration resolve CudaToolkitDir to
    rem empty. Repair it when missing (best-effort; needs admin - the
    rem version-specific env var above covers the non-admin case).
    reg query "HKLM\SOFTWARE\NVIDIA Corporation\GPU Computing Toolkit\CUDA\!CUDA_FOLDER!" /v InstallDir >nul 2>nul
    if !errorlevel! neq 0 (
        reg add "HKLM\SOFTWARE\NVIDIA Corporation\GPU Computing Toolkit\CUDA\!CUDA_FOLDER!" /v InstallDir /t REG_SZ /d "!CUDA_DIR!" /f >nul 2>&1
        echo [gpu] Repaired the CUDA registry InstallDir entry.
    )
)
exit /b 0

:purge_broken_cmake
rem Remove llama.cpp cmake build trees that were configured but never
rem finished (CMakeCache.txt present, generated project missing) - they
rem make every subsequent build fail with MSB1009.
for /d %%F in ("target\release\build\llama-cpp-sys-2-*") do (
    if exist "%%F\out\build\CMakeCache.txt" (
        if not exist "%%F\out\build\INSTALL.vcxproj" (
            echo [build] Removing a stale partial cmake build tree...
            rd /s /q "%%F\out\build"
        )
    )
)
exit /b 0

:copy_cuda_dlls
rem %1 = the CUDA toolkit root; copies the runtime DLLs the local-model
rem engine links against next to tag-gui.exe. CUDA 12 keeps them in
rem `bin`, CUDA 13 in `bin\x64` - check both. The file lists must come
rem from `dir /b`: a plain `for %%F in (glob)` expands against the
rem current directory, which has no matches.
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
if defined COPIED (
    echo [done] CUDA runtime DLLs copied next to the app.
) else (
    echo [gpu] WARNING: CUDA runtime DLLs not found in the toolkit -
    echo        the app may fail to start. Re-run install.bat.
)
exit /b 0

:fail
echo.
echo Installation did not complete. Fix the issue above and run
echo install.bat again - it is safe to re-run at any time.
echo.
pause
exit /b 1
