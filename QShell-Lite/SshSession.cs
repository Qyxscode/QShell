// 一个 SSH 会话 = 一个标签页 = 一条连接 + 一个终端控件。

using System;
using System.Drawing;
using System.IO;
using System.Reflection;
using System.Windows.Forms;
using Renci.SshNet;

namespace QShellLite
{
    internal sealed class SshSession : ISession
    {
        public HostEntry Entry { get; private set; }

        private readonly VtTerminal term;
        private readonly TerminalView view;
        private readonly Timer pumpTimer = new Timer();
        private readonly Timer resizeTimer = new Timer();

        private SshClient client;
        private SshClient jumpClient;
        private ForwardedPortLocal jumpForward;
        private ShellStream shell;
        private object channel;
        private MethodInfo channelResize;
        private bool disposed;
        private readonly Timer reconnectTimer = new Timer();
        private StreamWriter logWriter;
        private bool reconnecting;

        /// <summary>状态消息（连接成功/断开/错误）。</summary>
        public event Action<string> Message;

        /// <summary>由宿主设置的标签页。</summary>
        public TabPage Page { get; set; }

        public TerminalView View
        {
            get { return view; }
        }

        public bool IsConnected
        {
            get { return shell != null; }
        }

        /// <summary>断线后自动重连（来自 settings.auto_reconnect）。</summary>
        public bool AutoReconnect { get; set; }

        private bool logEnabled;

        /// <summary>记录会话输出（来自 settings.session_log_enabled）。中途打开会立即开始记录。</summary>
        public bool LogEnabled
        {
            get { return logEnabled; }
            set
            {
                logEnabled = value;
                if (value && shell != null)
                {
                    OpenLog();
                }
                else if (!value)
                {
                    CloseLog();
                }
            }
        }

        public SshSession(HostEntry entry, int cols, int rows)
        {
            Entry = entry;
            term = new VtTerminal(rows, cols);
            view = new TerminalView(term);
            view.Input = Send;
            view.SizeChangedHandlers = OnViewResized;

            pumpTimer.Interval = 25;
            pumpTimer.Tick += OnPump;

            // 拖完窗口再通知远端，否则 bash 会反复重绘、和本地状态错位
            resizeTimer.Interval = 400;
            resizeTimer.Tick += delegate
            {
                resizeTimer.Stop();
                SendWindowSize();
            };

            reconnectTimer.Interval = 3000;
            reconnectTimer.Tick += OnReconnectTick;
        }

        public bool Connect(out string error)
        {
            error = "";
            try
            {
                string targetHost = Entry.Host;
                int targetPort = Entry.Port;

                // 跳板机：先在跳板机上开一个本地端口转发到目标 22，再连这个本地端口
                if (!string.IsNullOrEmpty(Entry.ProxyHost))
                {
                    if (!string.IsNullOrEmpty(Entry.ProxyKey) && System.IO.File.Exists(Entry.ProxyKey))
                    {
                        jumpClient = new SshClient(Entry.ProxyHost, Entry.ProxyPort, Entry.ProxyUser,
                            new PrivateKeyFile(Entry.ProxyKey));
                    }
                    else
                    {
                        jumpClient = new SshClient(Entry.ProxyHost, Entry.ProxyPort, Entry.ProxyUser,
                            Entry.ProxyPwd ?? "");
                    }
                    jumpClient.Connect();

                    jumpForward = new ForwardedPortLocal("127.0.0.1", 0, Entry.Host, (uint)Entry.Port);
                    jumpClient.AddForwardedPort(jumpForward);
                    jumpForward.Start();

                    targetHost = "127.0.0.1";
                    targetPort = (int)jumpForward.BoundPort;
                }

                if (!string.IsNullOrEmpty(Entry.KeyPath) && System.IO.File.Exists(Entry.KeyPath))
                {
                    // 私钥认证（OpenSSH 格式和 PuTTY .ppk 都支持）
                    var keyFile = new PrivateKeyFile(Entry.KeyPath);
                    client = new SshClient(targetHost, targetPort, Entry.User, keyFile);
                }
                else
                {
                    client = new SshClient(targetHost, targetPort, Entry.User, Entry.Pwd ?? "");
                }
                client.Connect();
                shell = client.CreateShellStream("xterm-256color",
                    (uint)term.Cols, (uint)term.Rows, 0, 0, 8192);
                channel = GetChannel(shell);
                channelResize = GetResizeMethod(channel);
                OpenLog();
                pumpTimer.Start();
                return true;
            }
            catch (Exception ex)
            {
                error = ex.Message;
                Close();
                return false;
            }
        }

        public void Close()
        {
            pumpTimer.Stop();
            resizeTimer.Stop();
            reconnectTimer.Stop();
            CloseLog();
            try
            {
                if (shell != null) shell.Dispose();
            }
            catch { }
            shell = null;
            channel = null;
            channelResize = null;
            try
            {
                if (client != null)
                {
                    client.Disconnect();
                    client.Dispose();
                }
            }
            catch { }
            client = null;

            try
            {
                if (jumpForward != null)
                {
                    jumpForward.Stop();
                    jumpForward.Dispose();
                }
            }
            catch { }
            jumpForward = null;

            try
            {
                if (jumpClient != null)
                {
                    jumpClient.Disconnect();
                    jumpClient.Dispose();
                }
            }
            catch { }
            jumpClient = null;
        }

        public void Dispose()
        {
            if (disposed) return;
            disposed = true;
            Close();
        }

        // ---------------- 数据流 ----------------

        private void OnPump(object sender, EventArgs e)
        {
            if (disposed) return;

            ShellStream sh = shell;
            if (sh == null) return;

            // DataAvailable 在断线后只是返回 false，不会抛异常 —— 必须主动查连接状态
            if (client == null || !client.IsConnected)
            {
                OnLinkBroken();
                return;
            }

            try
            {
                var buf = new byte[8192];
                int budget = 262144;   // 每帧最多 256KB，避免大量输出把界面卡死
                while (sh.DataAvailable && budget > 0)
                {
                    int n = sh.Read(buf, 0, Math.Min(buf.Length, budget));
                    if (n <= 0) break;
                    budget -= n;
                    if (logWriter != null)
                    {
                        try
                        {
                            string raw = System.Text.Encoding.UTF8.GetString(buf, 0, n);
                            logWriter.Write(CleanForLog(raw));
                        }
                        catch { }
                    }
                    term.Feed(buf, 0, n);
                }
            }
            catch
            {
                OnLinkBroken();
            }
        }

        /// <summary>把文本发给远端（备忘录命令用）。</summary>
        public void SendText(string text)
        {
            if (string.IsNullOrEmpty(text)) return;
            Send(System.Text.Encoding.UTF8.GetBytes(text));
        }

        private void Send(byte[] data)
        {
            ShellStream sh = shell;
            if (sh == null) return;
            try
            {
                sh.Write(data, 0, data.Length);
                sh.Flush();
            }
            catch
            {
            }
        }

        private void OnViewResized()
        {
            // 用 ViewWidth（扣掉滚动条），否则远端会多画一列被滚动条遮住
            int cols = Math.Max(10, view.ViewWidth / view.CellWidth);
            int rows = Math.Max(3, view.Height / view.CellHeight);
            if (cols == term.Cols && rows == term.Rows) return;
            term.Resize(rows, cols);

            if (channelResize == null) return;
            resizeTimer.Stop();
            resizeTimer.Start();
        }

        private void SendWindowSize()
        {
            if (channelResize == null || channel == null) return;
            try
            {
                channelResize.Invoke(channel, new object[]
                {
                    (uint)term.Cols, (uint)term.Rows, (uint)0, (uint)0
                });
            }
            catch
            {
            }
        }

        // ---------------- SSH.NET 反射 ----------------

        // ---------------- 断线重连 ----------------

        private void OnLinkBroken()
        {
            if (disposed || reconnecting) return;
            if (shell == null) return;

            try
            {
                shell.Dispose();
            }
            catch { }
            shell = null;

            if (!AutoReconnect)
            {
                if (Message != null) Message("连接已断开");
                return;
            }

            reconnecting = true;
            if (Message != null) Message("连接断开，3 秒后自动重连...");
            reconnectTimer.Start();
        }

        private void OnReconnectTick(object sender, EventArgs e)
        {
            reconnectTimer.Stop();
            if (disposed) return;

            // 保留终端内容（term/view 不动），只重建 SSH 连接
            try
            {
                if (client != null)
                {
                    try { client.Dispose(); } catch { }
                    client = null;
                }
                if (jumpForward != null)
                {
                    try { jumpForward.Stop(); jumpForward.Dispose(); } catch { }
                    jumpForward = null;
                }
                if (jumpClient != null)
                {
                    try { jumpClient.Dispose(); } catch { }
                    jumpClient = null;
                }

                string targetHost = Entry.Host;
                int targetPort = Entry.Port;
                if (!string.IsNullOrEmpty(Entry.ProxyHost))
                {
                    if (!string.IsNullOrEmpty(Entry.ProxyKey) && System.IO.File.Exists(Entry.ProxyKey))
                    {
                        jumpClient = new SshClient(Entry.ProxyHost, Entry.ProxyPort, Entry.ProxyUser,
                            new PrivateKeyFile(Entry.ProxyKey));
                    }
                    else
                    {
                        jumpClient = new SshClient(Entry.ProxyHost, Entry.ProxyPort, Entry.ProxyUser,
                            Entry.ProxyPwd ?? "");
                    }
                    jumpClient.Connect();
                    jumpForward = new ForwardedPortLocal("127.0.0.1", 0, Entry.Host, (uint)Entry.Port);
                    jumpClient.AddForwardedPort(jumpForward);
                    jumpForward.Start();
                    targetHost = "127.0.0.1";
                    targetPort = (int)jumpForward.BoundPort;
                }

                client = new SshClient(targetHost, targetPort, Entry.User, Entry.Pwd ?? "");
                client.Connect();
                shell = client.CreateShellStream("xterm-256color",
                    (uint)term.Cols, (uint)term.Rows, 0, 0, 8192);
                channel = GetChannel(shell);
                channelResize = GetResizeMethod(channel);
                OpenLog();
                pumpTimer.Start();
                reconnecting = false;
                if (Message != null) Message("已重连");
            }
            catch (Exception ex)
            {
                if (Message != null) Message("重连失败: " + ex.Message + "，稍后重试");
                reconnectTimer.Start();   // 继续重试
            }
        }

        // ---------------- 会话日志 ----------------

        private static readonly System.Text.RegularExpressions.Regex CsiRegex =
            new System.Text.RegularExpressions.Regex(
                "\u001b\\[[0-9;?]*[ -/]*[@-~]",
                System.Text.RegularExpressions.RegexOptions.Compiled);

        private static readonly System.Text.RegularExpressions.Regex OscRegex =
            new System.Text.RegularExpressions.Regex(
                "\u001b\\][^\u0007\u001b]*(\u0007|\u001b\\\\)?",
                System.Text.RegularExpressions.RegexOptions.Compiled);

        /// <summary>去掉 ANSI 控制序列，只留可读文本（日志用）。</summary>
        private static string CleanForLog(string raw)
        {
            if (string.IsNullOrEmpty(raw)) return "";

            string text = CsiRegex.Replace(raw, "");
            text = OscRegex.Replace(text, "");
            text = text.Replace("\u001b(B", "").Replace("\u001b)0", "");
            text = text.Replace("\u001b=", "").Replace("\u001b>", "");
            text = text.Replace("\u001bM", "");
            // 去掉除 \n \r \t 外的控制字符
            var sb = new System.Text.StringBuilder(text.Length);
            foreach (char ch in text)
            {
                if (ch == '\n' || ch == '\r' || ch == '\t' || ch >= 0x20)
                {
                    sb.Append(ch);
                }
            }
            return sb.ToString();
        }

        private void OpenLog()
        {
            if (!logEnabled || logWriter != null) return;
            try
            {
                string dir = System.IO.Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                    "QShell", "logs");
                System.IO.Directory.CreateDirectory(dir);

                string safeName = (Entry.Name.Length > 0 ? Entry.Name : Entry.Host);
                foreach (char bad in System.IO.Path.GetInvalidFileNameChars())
                {
                    safeName = safeName.Replace(bad, '_');
                }
                string file = System.IO.Path.Combine(dir,
                    safeName + "_" + DateTime.Now.ToString("yyyyMMdd_HHmmss") + ".log");
                logWriter = new System.IO.StreamWriter(file, true, System.Text.Encoding.UTF8);
                logWriter.AutoFlush = true;
            }
            catch
            {
                logWriter = null;
            }
        }

        private void CloseLog()
        {
            if (logWriter == null) return;
            try
            {
                logWriter.Flush();
                logWriter.Dispose();
            }
            catch { }
            logWriter = null;
        }

        /// <summary>SSH.NET 2020 的 ShellStream 没有公开的窗口尺寸接口，反射拿私有 channel。</summary>
        private static object GetChannel(ShellStream stream)
        {
            try
            {
                FieldInfo field = typeof(ShellStream).GetField(
                    "_channel", BindingFlags.NonPublic | BindingFlags.Instance);
                if (field != null) return field.GetValue(stream);
            }
            catch { }
            return null;
        }

        private static MethodInfo GetResizeMethod(object ch)
        {
            if (ch == null) return null;
            try
            {
                return ch.GetType().GetMethod("SendWindowChangeRequest",
                    new Type[] { typeof(uint), typeof(uint), typeof(uint), typeof(uint) });
            }
            catch
            {
                return null;
            }
        }
    }
}
