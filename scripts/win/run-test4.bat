@echo off
setlocal
set SRC=%~dp0
set DST=%TEMP%\aiterm-win-test
set LOG=%SRC%win-test4-result.txt
mkdir "%DST%" 2>nul
copy /y "%SRC%aitermd.exe" "%DST%" >nul
copy /y "%SRC%aiterm.exe" "%DST%" >nul
cd /d "%DST%"
echo === round4: diagnostics %DATE% %TIME% === > "%LOG%"
taskkill /im aitermd.exe /f >nul 2>&1
del /q %USERPROFILE%\aiterm-proof.txt 2>nul
set AITERM_DEBUG=1
set AITERM_SHELL=cmd.exe
rem visible daemon console so debug lines can be screenshotted
start "aitermd-debug" "%DST%\aitermd.exe"
timeout /t 3 /nobreak >nul
aiterm.exe new-surface --title diag >> "%LOG%" 2>&1
timeout /t 6 /nobreak >nul
echo --- is shell child alive --- >> "%LOG%"
tasklist | findstr /i "cmd.exe aitermd" >> "%LOG%"
echo --- stdin proof: write file via injected command --- >> "%LOG%"
aiterm.exe send --surface surface:1 "echo PROOF_VIA_STDIN > %USERPROFILE%\aiterm-proof.txt" >> "%LOG%" 2>&1
aiterm.exe send-key --surface surface:1 Return >> "%LOG%" 2>&1
timeout /t 5 /nobreak >nul
if exist %USERPROFILE%\aiterm-proof.txt (
  echo PROOF FILE EXISTS: >> "%LOG%"
  type %USERPROFILE%\aiterm-proof.txt >> "%LOG%"
) else (
  echo PROOF FILE MISSING >> "%LOG%"
)
echo --- read-screen --- >> "%LOG%"
aiterm.exe read-screen --surface surface:1 >> "%LOG%" 2>&1
echo === DONE (daemon left running for console inspection) === >> "%LOG%"
exit
