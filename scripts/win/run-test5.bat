@echo off
setlocal
set SRC=%~dp0
set DST=%TEMP%\aiterm-win-test
set LOG=%SRC%win-test5-result.txt
mkdir "%DST%" 2>/dev/null
copy /y "%SRC%aitermd.exe" "%DST%" >/dev/null
copy /y "%SRC%aiterm.exe" "%DST%" >/dev/null
cd /d "%DST%"
echo === round5: DSR fix %DATE% %TIME% === > "%LOG%"
taskkill /im aitermd.exe /f >/dev/null 2>&1
del /q %USERPROFILE%\aiterm-proof.txt 2>/dev/null
start /b "" "%DST%\aitermd.exe"
timeout /t 3 /nobreak >/dev/null
aiterm.exe new-surface --title dsr-fix >> "%LOG%" 2>&1
timeout /t 6 /nobreak >/dev/null
echo --- screen after 6s (expect prompt) --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
aiterm.exe send --surface surface:1 "echo HELLO_WIN_FIXED && echo PROOF > %USERPROFILE%\aiterm-proof.txt" >> "%LOG%" 2>&1
aiterm.exe send-key --surface surface:1 Return >> "%LOG%" 2>&1
timeout /t 4 /nobreak >/dev/null
echo --- screen after echo --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
if exist %USERPROFILE%\aiterm-proof.txt (echo PROOF FILE EXISTS >> "%LOG%") else (echo PROOF FILE MISSING >> "%LOG%")
aiterm.exe close-surface surface:1 >> "%LOG%" 2>&1
taskkill /im aitermd.exe /f >/dev/null 2>&1
echo === DONE === >> "%LOG%"
exit
