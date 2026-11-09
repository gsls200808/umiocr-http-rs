#requires -version 5.1
<#
.SYNOPSIS
    构建并打包 umiocr-http-rs 的 Windows 发行包。

.DESCRIPTION
    1. cargo build --release
    2. 组装 dist/umiocr-http-rs-<版本>-<目标三元组>/（可执行文件、configs、web、README）
    3. 按需附带 ONNX Runtime 动态库与 models 目录
    4. 压缩为 zip

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\package.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -OrtLib D:\ort\onnxruntime.dll -ModelsDir D:\Umi-OCR\models

.EXAMPLE
    # 已经手动编译过，只重新打包
    powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -SkipBuild
#>
[CmdletBinding()]
param(
    # ONNX Runtime 动态库路径（onnxruntime.dll），指定后会复制到包根目录
    [string]$OrtLib = "",
    # 模型目录路径（其下应包含 configs.txt / *.onnx / keys.txt），复制为包内 models/
    [string]$ModelsDir = "",
    # 跳过 cargo build，直接用已有产物打包
    [switch]$SkipBuild,
    # 不生成 zip，只保留目录
    [switch]$NoZip
)

$ErrorActionPreference = 'Stop'

function Resolve-Tool {
    param([string]$Name, [string]$Fallback)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($Fallback -and (Test-Path $Fallback)) { return $Fallback }
    throw "未找到 $Name，请先安装 Rust 工具链（https://rustup.rs）并确认已在 PATH 中"
}

function Copy-Tree {
    param([string]$Source, [string]$Destination)
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item -Path (Join-Path $Source '*') -Destination $Destination -Recurse -Force
}

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargoExe = Resolve-Tool -Name 'cargo' -Fallback (Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe')
$rustcExe = Resolve-Tool -Name 'rustc' -Fallback (Join-Path $env:USERPROFILE '.cargo\bin\rustc.exe')

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$hostTriple = (& $rustcExe -vV | Select-String -Pattern '^host:\s*(\S+)').Matches[0].Groups[1].Value

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$binPath = Join-Path $targetDir 'release\umiocr-http-rs.exe'

Write-Host "==> 项目版本: $version ($hostTriple)"

if (-not $SkipBuild) {
    Write-Host '==> cargo build --release'
    & $cargoExe build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }
} else {
    Write-Host '==> 跳过编译（-SkipBuild）'
}

if (-not (Test-Path $binPath)) {
    throw "未找到构建产物 $binPath，请去掉 -SkipBuild 重新打包"
}

$pkgName = "umiocr-http-rs-$version-$hostTriple"
$distRoot = Join-Path $root 'dist'
$outDir = Join-Path $distRoot $pkgName

if (Test-Path $outDir) {
    try {
        Remove-Item -Recurse -Force $outDir -ErrorAction Stop
    } catch {
        # 旧目录被占用（资源管理器/杀软/索引服务）时，改用带时间戳的新目录
        $outDir = "$outDir-" + (Get-Date -Format 'yyyyMMdd-HHmmss')
        Write-Warning "无法清理旧目录，改用 $outDir"
    }
}
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

Write-Host "==> 组装 $pkgName"
Copy-Item $binPath $outDir
New-Item -ItemType Directory -Force -Path (Join-Path $outDir 'configs') | Out-Null
Copy-Item (Join-Path $root 'configs\application.yaml') (Join-Path $outDir 'configs')
Copy-Tree -Source (Join-Path $root 'web') -Destination (Join-Path $outDir 'web')
Copy-Item (Join-Path $root 'README.md') $outDir
Copy-Item (Join-Path $root 'THIRD_PARTY_NOTICES.md') $outDir

# ONNX Runtime 动态库：显式指定优先，其次项目根目录 / runtime 目录下的 onnxruntime.dll
$ortCandidate = $OrtLib
if (-not $ortCandidate) {
    foreach ($p in @((Join-Path $root 'onnxruntime.dll'), (Join-Path $root 'runtime\onnxruntime.dll'))) {
        if (Test-Path $p) { $ortCandidate = $p; break }
    }
}
if ($ortCandidate) {
    if (-not (Test-Path $ortCandidate)) { throw "ONNX Runtime 动态库不存在: $ortCandidate" }
    Copy-Item $ortCandidate (Join-Path $outDir 'onnxruntime.dll')
    Write-Host "    已附带 onnxruntime.dll <- $ortCandidate"
} else {
    Write-Warning '未附带 onnxruntime.dll，请在运行前自行放入包目录（或用 -OrtLib 指定）'
}

# 模型目录：显式指定优先，其次项目根目录下的 models
$modelsCandidate = $ModelsDir
if (-not $modelsCandidate -and (Test-Path (Join-Path $root 'models'))) {
    $modelsCandidate = Join-Path $root 'models'
}
if ($modelsCandidate) {
    if (-not (Test-Path $modelsCandidate)) { throw "模型目录不存在: $modelsCandidate" }
    Copy-Tree -Source $modelsCandidate -Destination (Join-Path $outDir 'models')
    Write-Host "    已附带 models <- $modelsCandidate"
} else {
    Write-Warning '未附带 models 目录，请在运行前自行放入包目录（或用 -ModelsDir 指定）'
}

if (-not $NoZip) {
    $zipPath = Join-Path $distRoot "$pkgName.zip"
    Write-Host '==> 生成压缩包'
    try {
        Compress-Archive -Path $outDir -DestinationPath $zipPath -Force -ErrorAction Stop
    } catch {
        # 旧 zip 被占用时换用带时间戳的文件名
        $zipPath = Join-Path $distRoot ("$pkgName-" + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.zip')
        Compress-Archive -Path $outDir -DestinationPath $zipPath -Force
    }
    $sizeMb = [math]::Round((Get-Item $zipPath).Length / 1MB, 1)
    Write-Host "==> 完成: $zipPath ($sizeMb MB)"
} else {
    Write-Host "==> 完成: $outDir"
}
