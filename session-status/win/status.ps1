<#
  Control script for the Claude session-status widget on Windows.

    status.ps1 build        -> compile the WPF widget (Release)
    status.ps1 float        -> start the always-on-top floating widget (idempotent)
    status.ps1 float-stop   -> stop the widget
    status.ps1 install      -> wire record.py into ~/.claude/settings.json hooks (non-destructive)
    status.ps1 uninstall    -> remove ONLY our hooks + clear state (leaves your other hooks alone)

  Code lives in this repo; runtime data lives in ~/.claude/session-status/.
#>
param([Parameter(Position = 0)][string]$Command = "help")

$ErrorActionPreference = "Stop"
$Dir     = $PSScriptRoot                                   # ...\session-status\win
$Proj    = Join-Path $Dir "ClaudeSessions\ClaudeSessions.csproj"
$Exe     = Join-Path $Dir "ClaudeSessions\bin\Release\net9.0-windows\ClaudeSessions.exe"
$Install = Join-Path $Dir "install.py"
$Data    = Join-Path $env:USERPROFILE ".claude\session-status"

function Find-Python {
    foreach ($c in @("python", "py")) {
        $p = Get-Command $c -ErrorAction SilentlyContinue
        if ($p) { return $p.Source }
    }
    throw "Python not found on PATH."
}
function Find-Dotnet {
    $p = Get-Command dotnet -ErrorAction SilentlyContinue
    if ($p) { return $p.Source }
    $fallback = "C:\Program Files\dotnet\dotnet.exe"
    if (Test-Path $fallback) { return $fallback }
    throw "dotnet SDK not found."
}

switch ($Command) {
    "build" {
        & (Find-Dotnet) build -c Release $Proj
    }
    "float" {
        if (Get-Process ClaudeSessions -ErrorAction SilentlyContinue) {
            "widget already running"; break
        }
        if (-not (Test-Path $Exe)) { "building first..."; & (Find-Dotnet) build -c Release $Proj }
        Start-Process $Exe
        "floating widget started (top-right, always on top). Stop with: status.ps1 float-stop"
    }
    "float-stop" {
        $p = Get-Process ClaudeSessions -ErrorAction SilentlyContinue
        if ($p) { $p | Stop-Process -Force; "widget stopped" } else { "not running" }
    }
    "install" {
        & (Find-Python) $Install install
        "`nStart a NEW Claude session (or send a prompt) to populate the widget."
    }
    "uninstall" {
        & (Find-Python) $Install uninstall
        $p = Get-Process ClaudeSessions -ErrorAction SilentlyContinue
        if ($p) { $p | Stop-Process -Force }
        $state = Join-Path $Data "state"
        if (Test-Path $state) { Remove-Item $state -Recurse -Force; "state cleared" }
        "done. Restart your Claude sessions to fully detach."
    }
    default {
        "usage: status.ps1 {build|float|float-stop|install|uninstall}"
    }
}
