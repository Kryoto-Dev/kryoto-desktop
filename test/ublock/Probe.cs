using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.WinForms;

// A real WebView2 runtime, the bundled extension, and deterministic pages.
// The fixture never downloads a file or depends on a live ad server.
class Probe {
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    static StreamWriter log;
    static void Assert(bool value, string message) { if (!value) throw new Exception(message); }
    [STAThread] static void Main(string[] args) {
        var root = Path.GetDirectoryName(typeof(Probe).Assembly.Location);
        log = new StreamWriter(Path.Combine(root, "result.txt"), false) { AutoFlush = true };
        Application.EnableVisualStyles();
        var form = new Form { Width = 900, Height = 650, ShowInTaskbar = false, Opacity = 0 };
        var view = new WebView2 { Dock = DockStyle.Fill };
        form.Controls.Add(view);
        form.Shown += async (s, e) => {
            try {
                var options = new CoreWebView2EnvironmentOptions { AreBrowserExtensionsEnabled = true };
                var env = await CoreWebView2Environment.CreateAsync(null, Path.Combine(root, "profile"), options);
                await view.EnsureCoreWebView2Async(env);
                var core = view.CoreWebView2;
                log.WriteLine("WebView2 " + env.BrowserVersionString);
                int adRequests = 0;
                core.AddWebResourceRequestedFilter("*", CoreWebView2WebResourceContext.All);
                core.WebResourceRequested += (a, b) => {
                    string uri = b.Request.Uri, text, mime;
                    if (uri == "https://vikingfile.com/f/kryoto-test") {
                        text = "<button id='download'>Download</button><script src='https://cdn.kryoto-test.example/download.js'></script><script src='https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js'></script><script src='https://challenges.cloudflare.com/turnstile/v0/api.js'></script>";
                        mime = "text/html";
                    } else if (uri.StartsWith("https://cdn.kryoto-test.example/")) {
                        text = "window.__DOWNLOAD_READY=true;document.getElementById('download').onclick=()=>{window.__CLICKED=true}"; mime = "application/javascript";
                    } else if (uri.Contains("googlesyndication.com/pagead/js/adsbygoogle.js")) {
                        adRequests++; text = "window.__AD_EXECUTED=true"; mime = "application/javascript";
                    } else if (uri.StartsWith("https://challenges.cloudflare.com/turnstile/")) {
                        text = "window.__CHECK_READY=true"; mime = "application/javascript";
                    } else if (uri.StartsWith("http:" ) || uri.StartsWith("https:")) {
                        text = ""; mime = "text/plain";
                    } else { return; }
                    b.Response = env.CreateWebResourceResponse(new MemoryStream(Encoding.UTF8.GetBytes(text)), 200, "OK", "Content-Type: " + mime);
                };
                var ext = await core.Profile.AddBrowserExtensionAsync(args[0]);
                Assert(ext.IsEnabled && ext.Name == "uBlock Origin", "Bundled uBlock did not enable");
                string session = null;
                var deadline = DateTime.UtcNow.AddSeconds(25);
                while (session == null && DateTime.UtcNow < deadline) {
                    var targets = Json.Deserialize<Dictionary<string, object>>(await core.CallDevToolsProtocolMethodAsync("Target.getTargets", "{}"));
                    foreach (Dictionary<string, object> target in (ArrayList)targets["targetInfos"]) {
                        if ((string)target["url"] != "chrome-extension://" + ext.Id + "/background.html") continue;
                        var attach = Json.Deserialize<Dictionary<string, object>>(await core.CallDevToolsProtocolMethodAsync("Target.attachToTarget", Json.Serialize(new { targetId = target["targetId"], flatten = true })));
                        session = (string)attach["sessionId"];
                    }
                    if (session == null) await Task.Delay(100);
                }
                Assert(session != null, "uBlock background did not start");
                bool ready = false;
                while (!ready && DateTime.UtcNow < deadline) {
                    var result = Json.Deserialize<Dictionary<string, object>>(await core.CallDevToolsProtocolMethodForSessionAsync(session, "Runtime.evaluate", Json.Serialize(new { expression = "globalThis.\u00b5Block?.readyToFilter === true", returnByValue = true })));
                    var value = (Dictionary<string, object>)result["result"];
                    ready = value.ContainsKey("value") && (bool)value["value"];
                    if (!ready) await Task.Delay(100);
                }
                Assert(ready, "Bundled filters did not become ready");
                await core.CallDevToolsProtocolMethodAsync("Target.detachFromTarget", Json.Serialize(new { sessionId = session }));
                var navigation = new TaskCompletionSource<bool>();
                core.NavigationCompleted += (a, b) => navigation.TrySetResult(b.IsSuccess);
                core.Navigate("https://vikingfile.com/f/kryoto-test");
                Assert(await navigation.Task, "Fixture did not load");
                await Task.Delay(500);
                Assert(await core.ExecuteScriptAsync("window.__AD_EXECUTED===true") == "false", "The advertising script executed");
                Assert(adRequests == 0, "Original ad request reached the network");
                Assert(await core.ExecuteScriptAsync("window.__DOWNLOAD_READY===true") == "true", "Legitimate third-party download script was blocked");
                Assert(await core.ExecuteScriptAsync("window.__CHECK_READY===true") == "true", "Cloudflare check script was blocked");
                await core.ExecuteScriptAsync("document.getElementById('download').click()");
                Assert(await core.ExecuteScriptAsync("window.__CLICKED===true") == "true", "Download button did not work");
                log.WriteLine("PASS: full uBlock ready; ad blocked; third-party download button and Cloudflare script work.");
            } catch (Exception ex) { log.WriteLine("FAIL: " + ex); Environment.ExitCode = 1; }
            finally { view.Dispose(); form.Close(); }
        };
        Application.Run(form);
        log.Close();
    }
}
