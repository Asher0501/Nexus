@echo off
ping -n 2 127.0.0.1 >nul
echo {"route":"ok","content":"node completed successfully"}
