<#
Instala o Nemux no Windows, só para o usuário atual (sem administrador): os
binários em %LOCALAPPDATA%\Programs\Nemux, um atalho no Menu Iniciar, o comando
`nemux` no PATH e uma entrada em "Aplicativos instalados", para desinstalar por lá.

Uso (da raiz do repositório):
  powershell -ExecutionPolicy Bypass -File scripts\install-windows.ps1             compila em release e instala
  powershell -ExecutionPolicy Bypass -File scripts\install-windows.ps1 -NoBuild    instala o que já está em target\release
  powershell -ExecutionPolicy Bypass -File scripts\install-windows.ps1 -Uninstall  remove o que este script instalou

Os destinos podem ser trocados (útil para testar sem mexer na sua instalação):
  NEMUX_INSTALL_DIR (padrão %LOCALAPPDATA%\Programs\Nemux),
  NEMUX_START_MENU_DIR (a pasta Programas do Menu Iniciar),
  NEMUX_NO_REGISTRY=1 (não mexe no PATH nem em "Aplicativos instalados").
#>
param(
    [switch]$NoBuild,
    [switch]$Uninstall
)
$ErrorActionPreference = "Stop"

function Default($value, $fallback) { if ($value) { $value } else { $fallback } }

$InstallDir = Default $env:NEMUX_INSTALL_DIR (Join-Path $env:LOCALAPPDATA "Programs\Nemux")
$StartMenuDir = Default $env:NEMUX_START_MENU_DIR ([Environment]::GetFolderPath("Programs"))
$UseRegistry = $env:NEMUX_NO_REGISTRY -ne "1"
$Shortcut = Join-Path $StartMenuDir "Nemux.lnk"
$UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nemux"
$Exes = "nemux-app.exe", "nemux-mcp.exe"

$Repo = Split-Path -Parent $PSScriptRoot
$Release = Join-Path $Repo "target\release"

# Avisa os programas abertos (Explorer etc.) que o PATH mudou.
function Send-EnvironmentChange {
    if (-not ("Nemux.Env" -as [type])) {
        Add-Type -Namespace Nemux -Name Env -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint msg, UIntPtr wParam,
    string lParam, uint flags, uint timeout, out UIntPtr result);
'@
    }
    $result = [UIntPtr]::Zero
    # HWND_BROADCAST, WM_SETTINGCHANGE, SMTO_ABORTIFHUNG
    [void][Nemux.Env]::SendMessageTimeout([IntPtr]0xffff, 0x1a, [UIntPtr]::Zero,
        "Environment", 2, 5000, [ref]$result)
}

# O PATH do usuário como está no registro, sem expandir os %VARIÁVEIS%: gravar o
# expandido trocaria, por exemplo, %USERPROFILE% pelo caminho de hoje.
function Get-UserPath {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment")
    try {
        $key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    } finally { $key.Close() }
}

function Set-UserPath([string[]]$entries) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
    try {
        $key.SetValue("Path", ($entries -join ";"), [Microsoft.Win32.RegistryValueKind]::ExpandString)
    } finally { $key.Close() }
    Send-EnvironmentChange
}

function Split-Path-Entries([string]$path) {
    @($path -split ";" | Where-Object { $_ -ne "" })
}

function Test-SameDir([string]$a, [string]$b) {
    [Environment]::ExpandEnvironmentVariables($a).TrimEnd("\") -ieq $b.TrimEnd("\")
}

# Apaga as cópias antigas que ficaram de uma atualização com o .exe em uso.
function Remove-OldCopies {
    Get-ChildItem -LiteralPath $InstallDir -Filter "*.old*" -File -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }
}

# O Windows não deixa sobrescrever um .exe que está rodando (o nemux-mcp de uma
# sessão do Claude, o próprio Nemux aberto), mas deixa trocar o nome dele: o antigo
# vira .old e é apagado na próxima instalação.
function Install-File([string]$source, [string]$dest) {
    if (Test-Path -LiteralPath $dest) {
        try {
            Remove-Item -LiteralPath $dest -Force
        } catch {
            $old = "$dest.old-$(Get-Date -Format yyyyMMddHHmmss)"
            Rename-Item -LiteralPath $dest -NewName (Split-Path -Leaf $old)
        }
    }
    Copy-Item -LiteralPath $source -Destination $dest
}

function Test-InUse([string]$file) {
    try {
        [IO.File]::Open($file, "Open", "ReadWrite", "None").Close()
        $false
    } catch { $true }
}

if ($Uninstall) {
    # Confere antes de apagar qualquer coisa: com um .exe em uso, a pasta ficaria
    # pela metade.
    $inUse = @(Get-ChildItem -LiteralPath $InstallDir -Filter "*.exe*" -File -ErrorAction SilentlyContinue |
        Where-Object { Test-InUse $_.FullName })
    if ($inUse) {
        Write-Error ("Em uso: $($inUse.Name -join ', '). Feche o Nemux e as sessões do Claude " +
            "que usam o nemux-mcp e rode -Uninstall de novo.")
    }
    if ($UseRegistry) {
        $entries = Split-Path-Entries (Get-UserPath)
        $kept = @($entries | Where-Object { -not (Test-SameDir $_ $InstallDir) })
        if ($kept.Count -ne $entries.Count) {
            Set-UserPath $kept
            Write-Host "Removido do PATH: $InstallDir"
        }
        if (Test-Path $UninstallKey) {
            Remove-Item $UninstallKey -Recurse -Force
            Write-Host "Removido de Aplicativos instalados"
        }
    }
    if (Test-Path -LiteralPath $Shortcut) {
        Remove-Item -LiteralPath $Shortcut -Force
        Write-Host "Removido: $Shortcut"
    }
    # Só apaga a pasta se ela for mesmo a do Nemux.
    if (Test-Path -LiteralPath (Join-Path $InstallDir "nemux-app.exe")) {
        Remove-Item -LiteralPath $InstallDir -Recurse -Force
        Write-Host "Removido: $InstallDir"
    }
    Write-Host "Os seus dados ficam onde estavam: $env:APPDATA\nemux e $env:LOCALAPPDATA\nemux."
    exit 0
}

if (-not $NoBuild) {
    Write-Host "Compilando em release..."
    cargo build --release --manifest-path (Join-Path $Repo "Cargo.toml") -p nemux-app -p nemux-mcp
    if ($LASTEXITCODE -ne 0) {
        Write-Error ("A compilação falhou. Se o erro for de acesso negado ao nemux-mcp.exe, " +
            "feche as sessões do Claude que usam o de target\release.")
    }
}

foreach ($exe in $Exes) {
    if (-not (Test-Path -LiteralPath (Join-Path $Release $exe))) {
        Write-Error "Falta $Release\$($exe): rode sem -NoBuild."
    }
}

# Os binários ficam lado a lado: "Uso do Claude na barra" procura o nemux-mcp junto
# do nemux-app. O ConPTY vai embutido no nemux-app.exe.
Write-Host "Instalando em $InstallDir..."
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Remove-OldCopies
foreach ($exe in $Exes) {
    Install-File (Join-Path $Release $exe) (Join-Path $InstallDir $exe)
}
# `nemux` no terminal abre o app sem prender o terminal.
Set-Content -LiteralPath (Join-Path $InstallDir "nemux.cmd") -Encoding ascii `
    -Value "@start `"`" `"%~dp0nemux-app.exe`" %*"
# Cópia deste script, para desinstalar por "Aplicativos instalados".
Install-File $PSCommandPath (Join-Path $InstallDir "install-windows.ps1")

New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($Shortcut)
$link.TargetPath = Join-Path $InstallDir "nemux-app.exe"
$link.WorkingDirectory = $env:USERPROFILE
$link.IconLocation = "$(Join-Path $InstallDir 'nemux-app.exe'),0"
$link.Description = "Terminais em abas com uma memória que as sessões do Claude Code compartilham"
$link.Save()

if ($UseRegistry) {
    $entries = Split-Path-Entries (Get-UserPath)
    if (-not ($entries | Where-Object { Test-SameDir $_ $InstallDir })) {
        Set-UserPath ($entries + $InstallDir)
        $pathAdded = $true
    }

    $version = (Select-String -LiteralPath (Join-Path $Repo "Cargo.toml") `
        -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $size = (Get-ChildItem -LiteralPath $InstallDir -File | Measure-Object Length -Sum).Sum
    $uninstallCmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File " +
        "`"$(Join-Path $InstallDir 'install-windows.ps1')`" -Uninstall"
    New-Item -Path $UninstallKey -Force | Out-Null
    $values = @{
        DisplayName     = "Nemux"
        DisplayVersion  = $version
        Publisher       = "Nemux"
        DisplayIcon     = "$(Join-Path $InstallDir 'nemux-app.exe'),0"
        InstallLocation = $InstallDir
        UninstallString = $uninstallCmd
    }
    foreach ($name in $values.Keys) {
        New-ItemProperty -Path $UninstallKey -Name $name -Value $values[$name] -PropertyType String -Force | Out-Null
    }
    foreach ($name in "NoModify", "NoRepair") {
        New-ItemProperty -Path $UninstallKey -Name $name -Value 1 -PropertyType DWord -Force | Out-Null
    }
    New-ItemProperty -Path $UninstallKey -Name EstimatedSize -Value ([int]($size / 1KB)) `
        -PropertyType DWord -Force | Out-Null
}

$mcp = Join-Path $InstallDir "nemux-mcp.exe"
Write-Host ""
Write-Host "Nemux instalado:"
Write-Host "  binários   $InstallDir"
Write-Host "  menu       $Shortcut"
if ($UseRegistry) {
    Write-Host "  comando    nemux$(if ($pathAdded) { ' (abra um terminal novo para o PATH valer)' })"
}
Write-Host ""
Write-Host "Para a memória no Claude Code (uma vez, vale para todos os projetos):"
Write-Host "  claude mcp add --scope user nemux -- `"$mcp`""
Write-Host "(se já estava registrado por outro caminho, antes: claude mcp remove nemux -s user)"
Write-Host "`"Uso do Claude na barra`" (engrenagem) já acha o nemux-mcp de $InstallDir; se"
Write-Host "ligou antes por outro caminho, desmarque e marque de novo."
