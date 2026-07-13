@echo off
REM Cross-compile C to x86_64-linux-gnu via zig cc.
REM The cc crate passes --target=x86_64-unknown-linux-gnu which zig
REM doesn't understand. We filter it out and use zig's own target triple.
REM Prerequisite: zig must be on PATH (winget install zig.zig).
setlocal enabledelayedexpansion
set ARGS=
:loop
if "%~1"=="" goto done
set "ARG=%~1"
REM cc crate passes this; zig uses its own target spelling
if "!ARG!"=="--target=x86_64-unknown-linux-gnu" shift && goto loop
REM cc crate also passes -m64; zig doesn't need it
if "!ARG!"=="-m64" shift && goto loop
set "ARGS=!ARGS! !ARG!"
shift
goto loop
:done
zig cc -target x86_64-linux-gnu %ARGS%
