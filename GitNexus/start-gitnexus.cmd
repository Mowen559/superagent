@echo off
setlocal
powershell -ExecutionPolicy Bypass -File "%~dp0scripts\start-gitnexus.ps1"
exit /b %errorlevel%
