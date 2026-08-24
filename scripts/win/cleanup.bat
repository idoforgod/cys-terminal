@echo off
taskkill /im aitermd.exe /f >/dev/null 2>&1
rmdir /s /q "%TEMP%\aiterm-win-test" 2>/dev/null
del /q %USERPROFILE%\aiterm-proof.txt 2>/dev/null
echo cleaned > "%~dp0win-cleanup-done.txt"
exit
