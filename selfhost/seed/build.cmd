@echo off
rem Copyright (c) 2026 Jurjen Stellingwerff
rem SPDX-License-Identifier: LGPL-3.0-or-later
rem
rem Builds the loft bytecode runner from this seed with MSVC alone: no cargo, no rustc, no loft
rem (@PLN91, doc/claude/plans/91-self-host/rust-free-build.md, level 1).  build.sh is the POSIX
rem and MinGW form of the same build.  `cl` is found on PATH, else through vswhere.
rem
rem     build.cmd
rem     set OUT=C:\tools\loft-run.exe & build.cmd
rem
rem The flags are `loft --native-c`'s for `cl` (c_driver.rs msvc_args): C11 mode, the unit and the
rem shim compiled together under whole-program optimisation (/GL, /LTCG), Winsock, the certificate
rem store and advapi32 linked, the main thread given a Linux one's 8 MiB stack.
setlocal
if /i "%~1"=="--help" goto usage
if /i "%~1"=="-h" goto usage
cd /d "%~dp0"
if "%OUT%"=="" set "OUT=run_image_run.exe"
if "%CFLAGS%"=="" set "CFLAGS=/O2"
where cl >nul 2>nul
if not errorlevel 1 goto build
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" goto nomsvc
set "VSDIR="
for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSDIR=%%i"
if not defined VSDIR goto nomsvc
call "%VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul
where cl >nul 2>nul
if errorlevel 1 goto nomsvc
:build
set "OBJ=%TEMP%\loft_seed_obj_%RANDOM%"
mkdir "%OBJ%"
cl /nologo /std:c11 /W0 %CFLAGS% /GL /DLOFT_BIND_LIBS /I gen gen\run_image_run.c selfhost\crt\shim\crt.c /Fo"%OBJ%\\" /Fe:"%OUT%" /link /LTCG /STACK:8388608 ws2_32.lib crypt32.lib advapi32.lib >"%OBJ%\cl.log"
set "RC=%ERRORLEVEL%"
if not "%RC%"=="0" type "%OBJ%\cl.log"
rmdir /s /q "%OBJ%"
exit /b %RC%
:nomsvc
echo build.cmd: no MSVC `cl` on PATH and none vswhere finds; open a Developer Command Prompt, or build with MinGW gcc through build.sh 1>&2
exit /b 2
:usage
echo usage: [set CFLAGS=/O2] [set OUT=run_image_run.exe] build.cmd
echo builds the loft bytecode runner from this seed with MSVC alone
exit /b 0
