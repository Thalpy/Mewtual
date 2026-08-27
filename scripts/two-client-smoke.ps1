# Two independent catcomsctl processes over a real loopback TCP socket.
# Optional test tooling only: no dependency is added to any Rust crate.
[CmdletBinding()]
param(
    [ValidateRange(1, 65535)]
    [int]$Port = 39090,
    [string]$Artifacts = "target/two-client-smoke/windows",
    [string]$Binary = "target/debug/catcomsctl.exe",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repo
$artifactInput = if ([System.IO.Path]::IsPathRooted($Artifacts)) { $Artifacts } else { Join-Path $repo $Artifacts }
$binaryInput = if ([System.IO.Path]::IsPathRooted($Binary)) { $Binary } else { Join-Path $repo $Binary }
$artifactPath = [System.IO.Path]::GetFullPath($artifactInput)
$binaryPath = [System.IO.Path]::GetFullPath($binaryInput)
New-Item -ItemType Directory -Force -Path $artifactPath | Out-Null

$invite = Join-Path $artifactPath "invite.txt"
$aliceLog = Join-Path $artifactPath "alice.log"
$bobLog = Join-Path $artifactPath "bob.log"
$manifest = Join-Path $artifactPath "manifest.txt"
Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $invite, $aliceLog, $bobLog, $manifest

if (-not $SkipBuild) {
    & cargo build -p catcomsctl
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
}
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "catcomsctl binary not found: $binaryPath"
}

$commit = (& git rev-parse HEAD 2>$null)
if (-not $commit) { $commit = "unknown" }
@(
    "commit=$commit"
    "scenario=join-and-catch-up-loopback"
    "started_at=$([DateTime]::UtcNow.ToString('o'))"
    "port=$Port"
    "phase=starting-alice"
) | Set-Content -Encoding utf8 -LiteralPath $manifest

$alice = $null
$bob = $null
try {
    $aliceArgs = @(
        "serve", "--port", "$Port", "--host", "127.0.0.1",
        "--invite-file", $invite
    )
    $alice = Start-Process -FilePath $binaryPath -ArgumentList $aliceArgs -PassThru `
        -WindowStyle Hidden -RedirectStandardOutput $aliceLog -RedirectStandardError "$aliceLog.err"

    Add-Content -LiteralPath $manifest -Value "phase=waiting-for-invite"
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Test-Path -LiteralPath $invite) -and (Get-Item -LiteralPath $invite).Length -gt 0) { break }
        if ($alice.HasExited) { throw "Alice exited before writing the invite; see $aliceLog" }
        Start-Sleep -Milliseconds 50
        $alice.Refresh()
    }
    if (-not (Test-Path -LiteralPath $invite) -or (Get-Item -LiteralPath $invite).Length -eq 0) {
        throw "Timed out waiting for Alice's invite; see $aliceLog"
    }

    Add-Content -LiteralPath $manifest -Value "phase=bob-joining"
    $bobArgs = @("join", "--invite-file", $invite)
    $bob = Start-Process -FilePath $binaryPath -ArgumentList $bobArgs -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $bobLog -RedirectStandardError "$bobLog.err"
    if (-not $bob.WaitForExit(45000)) {
        Stop-Process -Id $bob.Id -Force
        throw "Bob timed out; see $bobLog"
    }
    Add-Content -LiteralPath $manifest -Value "bob_exit=$($bob.ExitCode)"
    if ($bob.ExitCode -ne 0) { throw "Bob failed to join; see $bobLog and $aliceLog" }

    $bobOutput = Get-Content -Raw -LiteralPath $bobLog
    if (-not $bobOutput.Contains("[OK] joined and converged over libp2p")) {
        throw "Bob exited without the convergence marker; see $bobLog"
    }
    if (-not $bobOutput.Contains("Welcome! You joined a Mewtual server over libp2p.")) {
        throw "Bob did not receive Alice's encrypted channel message; see $bobLog"
    }

    Add-Content -LiteralPath $manifest -Value "phase=complete"
    Add-Content -LiteralPath $manifest -Value "outcome=passed"
    Add-Content -LiteralPath $manifest -Value "alice_exit=terminated-by-harness"
    Write-Host "PASS: two independent clients joined and converged over loopback TCP"
    Write-Host "Artifacts: $artifactPath"
}
catch {
    # Keep the manifest useful without copying exception text that may contain a path, invite, or
    # other unreviewed value into a CI artifact. The separate bounded logs carry operational detail.
    Add-Content -LiteralPath $manifest -Value "outcome=failed"
    Add-Content -LiteralPath $manifest -Value "failed_phase=$((Get-Content -LiteralPath $manifest | Select-String '^phase=' | Select-Object -Last 1).Line.Substring(6))"
    throw
}
finally {
    Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $invite
    if ($null -ne $bob) {
        $bob.Refresh()
        if (-not $bob.HasExited) { Stop-Process -Id $bob.Id -Force }
        $bob.Dispose()
    }
    if ($null -ne $alice) {
        $alice.Refresh()
        if (-not $alice.HasExited) { Stop-Process -Id $alice.Id -Force }
        $alice.Dispose()
    }
    if (Test-Path -LiteralPath $manifest) {
        Add-Content -LiteralPath $manifest -Value "finished_at=$([DateTime]::UtcNow.ToString('o'))"
    }
}
