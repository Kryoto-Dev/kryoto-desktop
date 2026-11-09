# Windows test programs import TaskDialogIndirect from comctl32, which only
# exists with the Common Controls v6 manifest. The app gets one from Tauri;
# a test program without one stops at once with 0xc0000139
# (STATUS_ENTRYPOINT_NOT_FOUND). Build the tests, then put a manifest beside
# each test program. Run from src-tauri, before `cargo test`.
$ErrorActionPreference = 'Stop'
$manifest = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
</assembly>
'@
$exes = cargo test --locked --no-run --message-format=json |
  ForEach-Object { try { $_ | ConvertFrom-Json } catch { $null } } |
  Where-Object { $_ -and $_.reason -eq 'compiler-artifact' -and $_.executable -and $_.profile.test } |
  ForEach-Object { $_.executable }
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
foreach ($exe in $exes) {
  Set-Content -Path "$exe.manifest" -Value $manifest -Encoding utf8
  # Windows caches what an exe needs; a newer exe makes it read the manifest.
  (Get-Item $exe).LastWriteTime = Get-Date
  Write-Host "manifest beside $exe"
}
