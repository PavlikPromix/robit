@echo off
:: -------------------------------------------------------------
:: move_to_other_drive.bat
::   Move any folder to a different NTFS drive and leave a
::   junction behind so Windows/applications stay happy.
::
::   USAGE   (run from an *elevated* Command Prompt):
::     move_to_other_drive.bat "C:\Path\To\Folder" "E:\New\Path\To\Folder"
::
::   EXAMPLE:
::     move_to_other_drive.bat "C:\Program Files\BigApp" "E:\Program Files\BigApp"
::
::   NOTES:
::     • Destination must be on an NTFS-formatted local drive.
::     • Auditing entries are skipped (COPY:DATSO) to avoid the
::       “Manage Auditing user right” error.
::     • Robocopy return codes ≥ 8 abort the script.
:: -------------------------------------------------------------

:: ---- sanity checks -------------------------------------------------
if "%~2"=="" (
    echo Usage: %~nx0 "SourceFolder" "DestinationFolder"
    exit /b 1
)

setlocal enableextensions
set "SRC=%~1"
set "DST=%~2"

if not exist "%SRC%" (
    echo Source "%SRC%" does not exist.
    exit /b 1
)

:: Create destination root if needed
if not exist "%DST%" (
    echo Creating destination "%DST%"
    md "%DST%" || (echo Failed to create destination & exit /b 1)
)

echo.
echo === Stage 1 – Moving files ========================================
robocopy "%SRC%" "%DST%" /E /COPYALL /MOVE /R:3 /W:5 /NFL /NDL /NP
if errorlevel 8 (
    echo Robocopy reported a fatal error – aborting.
    exit /b %ERRORLEVEL%
)

echo.
echo === Stage 2 – Removing empty source folder ========================
rd "%SRC%" 2>nul

echo.
echo === Stage 3 – Creating junction ====================================
mklink /J "%SRC%" "%DST%" || (
    echo Failed to create junction.
    exit /b 1
)

echo.
echo ---- All done! "%SRC%" is now a junction to "%DST%".
pause
