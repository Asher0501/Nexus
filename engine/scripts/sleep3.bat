@echo off
ping -n 4 127.0.0.1 >nul
echo {"route":"ok","content":"done"}
