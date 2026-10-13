$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('kryoto-ublock-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$sdk = Join-Path $testRoot 'sdk.zip'
Invoke-WebRequest 'https://api.nuget.org/v3-flatcontainer/microsoft.web.webview2/1.0.4258.31/microsoft.web.webview2.1.0.4258.31.nupkg' -OutFile $sdk
if ((Get-FileHash -LiteralPath $sdk -Algorithm SHA256).Hash.ToLowerInvariant() -ne '56f7f4b8bf9aee4b8efefbbdd4f67d5f74ebd1b100ed0806da71bf76af481aa9') { throw 'WebView2 SDK checksum differs.' }
Expand-Archive -LiteralPath $sdk -DestinationPath (Join-Path $testRoot 'sdk')
Copy-Item -LiteralPath (Join-Path $testRoot 'sdk/lib/net462/Microsoft.Web.WebView2.Core.dll') -Destination $testRoot
Copy-Item -LiteralPath (Join-Path $testRoot 'sdk/lib/net462/Microsoft.Web.WebView2.WinForms.dll') -Destination $testRoot
Copy-Item -LiteralPath (Join-Path $testRoot 'sdk/runtimes/win-x64/native/WebView2Loader.dll') -Destination $testRoot
$compiler = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
$executable = Join-Path $testRoot 'Probe.exe'
& $compiler /nologo /target:winexe ('/out:' + $executable) /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Web.Extensions.dll ('/r:' + (Join-Path $testRoot 'Microsoft.Web.WebView2.Core.dll')) ('/r:' + (Join-Path $testRoot 'Microsoft.Web.WebView2.WinForms.dll')) (Join-Path $repoRoot 'test/ublock/Probe.cs')
if ($LASTEXITCODE -ne 0) { throw 'WebView2 regression helper did not compile.' }
$extension = Join-Path $repoRoot 'src-tauri/resources/ublock'
$process = Start-Process -FilePath $executable -ArgumentList ('"' + $extension + '"') -WindowStyle Hidden -PassThru
if (-not $process.WaitForExit(60000)) { $process.Kill(); throw 'WebView2 extension test timed out.' }
Get-Content -LiteralPath (Join-Path $testRoot 'result.txt')
if ($process.ExitCode -ne 0) { throw 'WebView2 extension regression failed.' }
