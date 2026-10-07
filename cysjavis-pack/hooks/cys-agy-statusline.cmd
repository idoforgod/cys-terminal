@echo off
set NoDefaultCurrentDirectoryInExePath=1
cys usage-report-stdin --agy 2>nul
exit /b 0
