// spike 主窗口：连 SSH.NET，喂给 VtTerminal，显示在 TerminalView 上。
// 目标只是验证解析+渲染够不够用，不做多标签/配置读取。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.Reflection;
using System.Text;
using System.Windows.Forms;
using Renci.SshNet;

namespace QShellLite.Spike
{
    internal sealed class SpikeForm : Form
    {
        private readonly VtTerminal term = new VtTerminal(30, 110);
        private readonly TerminalView view;
        private readonly TextBox hostBox = new TextBox();
        private readonly TextBox portBox = new TextBox();
        private readonly TextBox userBox = new TextBox();
        private readonly TextBox pwdBox = new TextBox();
        private readonly Button connectButton = new Button();
        private readonly Label status = new Label();
        private readonly Panel top = new Panel();
        private readonly Panel searchPanel = new Panel();
        private readonly TextBox searchBox = new TextBox();
        private readonly Label searchCount = new Label();
        private readonly Button searchPrev = new Button();
        private readonly Button searchNext = new Button();
        private readonly Timer pumpTimer = new Timer();
        private readonly Timer resizeTimer = new Timer();

        private SshClient client;
        private ShellStream shell;
        private object channel;                 // IChannelSession 是 internal，只能存 object
        private MethodInfo channelResize;       // SendWindowChangeRequest
        private DateTime lastResizeSent = DateTime.MinValue;
        private System.IO.FileStream rawDump;
        private System.IO.FileStream txDump;
        private readonly bool dumpRaw;

        public SpikeForm(bool demo, bool dump, string replayPath)
        {
            dumpRaw = dump;
            Text = "QShell Lite VT Spike";
            ClientSize = new Size(1000, 620);
            StartPosition = FormStartPosition.CenterScreen;
            Font = new Font("Segoe UI", 9f);

            top.Padding = new Padding(6);
            Controls.Add(top);

            hostBox.Text = "127.0.0.1";
            hostBox.Location = new Point(6, 8);
            hostBox.Size = new Size(180, 24);
            top.Controls.Add(hostBox);

            portBox.Text = "22";
            portBox.Location = new Point(192, 8);
            portBox.Size = new Size(50, 24);
            top.Controls.Add(portBox);

            userBox.Text = "root";
            userBox.Location = new Point(248, 8);
            userBox.Size = new Size(120, 24);
            top.Controls.Add(userBox);

            pwdBox.UseSystemPasswordChar = true;
            pwdBox.Location = new Point(374, 8);
            pwdBox.Size = new Size(160, 24);
            top.Controls.Add(pwdBox);

            connectButton.Text = "连接";
            connectButton.Location = new Point(542, 6);
            connectButton.Size = new Size(70, 26);
            connectButton.Click += OnConnect;
            top.Controls.Add(connectButton);

            status.TextAlign = ContentAlignment.MiddleLeft;
            // 显示 exe 编译时间来确认跑的是哪个版本
            string built = "?";
            try
            {
                built = System.IO.File.GetLastWriteTime(Application.ExecutablePath).ToString("HH:mm:ss");
            }
            catch { }
            status.Text = "未连接  (built " + built + ")";
            Controls.Add(status);

            view = new TerminalView(term);
            view.Input = OnTerminalInput;
            view.SizeChangedHandlers = OnTerminalResized;
            view.SearchRequested += ShowSearch;
            Controls.Add(view);

            // 搜索栏（默认隐藏，Ctrl+F 打开）
            searchPanel.Visible = false;
            Controls.Add(searchPanel);

            var searchLabel = new Label();
            searchLabel.Text = "查找:";
            searchLabel.AutoSize = true;
            searchLabel.Location = new Point(6, 9);
            searchPanel.Controls.Add(searchLabel);

            searchBox.Location = new Point(48, 6);
            searchBox.Size = new Size(240, 24);
            searchBox.TextChanged += delegate
            {
                int count = view.Search(searchBox.Text);
                UpdateSearchCount(count);
            };
            searchBox.KeyDown += OnSearchKeyDown;
            searchPanel.Controls.Add(searchBox);

            searchCount.AutoSize = true;
            searchCount.Location = new Point(296, 10);
            searchPanel.Controls.Add(searchCount);

            searchPrev.Text = "上一个";
            searchPrev.Size = new Size(64, 26);
            searchPrev.Location = new Point(390, 5);
            searchPrev.Click += delegate { view.NextHit(false); UpdateSearchCount(view.SearchHitCount); };
            searchPanel.Controls.Add(searchPrev);

            searchNext.Text = "下一个";
            searchNext.Size = new Size(64, 26);
            searchNext.Location = new Point(460, 5);
            searchNext.Click += delegate { view.NextHit(true); UpdateSearchCount(view.SearchHitCount); };
            searchPanel.Controls.Add(searchNext);

            term.TitleChanged += OnTitleChanged;
            term.ClipboardRequest += OnClipboardRequest;

            pumpTimer.Interval = 25;
            pumpTimer.Tick += OnPump;
            pumpTimer.Start();

            // 拖动窗口时不要每次都通知远端：bash 每收到一次 SIGWINCH 就会重绘当前行，
            // 连续重绘会和本地状态错位。等拖完 400ms 再发一次。
            resizeTimer.Interval = 400;
            resizeTimer.Tick += delegate
            {
                resizeTimer.Stop();
                SendWindowResize();
            };

            FormClosing += OnClosing;

            // 默认记录原始字节，排查问题用；--nodump 可关闭
            if (dump)
            {
                // 优先写在 exe 同目录（共享文件夹里跑也能被宿主读到），失败再退回 TEMP
                string target = null;
                try
                {
                    string exeDir = System.IO.Path.GetDirectoryName(Application.ExecutablePath);
                    target = System.IO.Path.Combine(exeDir, "vtsplike-raw.bin");
                    rawDump = new System.IO.FileStream(target,
                        System.IO.FileMode.Create, System.IO.FileAccess.Write);
                }
                catch
                {
                    try
                    {
                        target = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "vtsplike-raw.bin");
                        rawDump = new System.IO.FileStream(target,
                            System.IO.FileMode.Create, System.IO.FileAccess.Write);
                    }
                    catch
                    {
                        target = null;
                    }
                }
                if (target != null)
                {
                    status.Text = "记录到: " + target;
                }

                // 同时记录发出去的字节（键盘），用来区分"没发出去"和"远端没回显"
                try
                {
                    string dir = System.IO.Path.GetDirectoryName(Application.ExecutablePath);
                    txDump = new System.IO.FileStream(
                        System.IO.Path.Combine(dir, "vtsplike-tx.bin"),
                        System.IO.FileMode.Create, System.IO.FileAccess.Write);
                }
                catch
                {
                    try
                    {
                        txDump = new System.IO.FileStream(
                            System.IO.Path.Combine(System.IO.Path.GetTempPath(), "vtsplike-tx.bin"),
                            System.IO.FileMode.Create, System.IO.FileAccess.Write);
                    }
                    catch { }
                }
            }

            if (demo)
            {
                LoadDemo();
            }

            if (!string.IsNullOrEmpty(replayPath) && System.IO.File.Exists(replayPath))
            {
                byte[] bytes = System.IO.File.ReadAllBytes(replayPath);
                term.Feed(bytes, 0, bytes.Length);
                status.Text = "回放: " + replayPath + " (" + bytes.Length + " 字节)";
            }
        }

        /// <summary>fed 一段典型 ANSI 序列，用来验证解析和渲染（不连服务器）。</summary>
        private void LoadDemo()
        {
            var sb = new System.Text.StringBuilder();
            sb.Append("\x1b[2J\x1b[H");
            sb.Append("\x1b[1;36mQShell Lite VT spike — 渲染自检\x1b[0m\r\n\r\n");
            sb.Append("\x1b[1;32m粗体绿\x1b[0m  \x1b[31m红\x1b[0m  \x1b[33m黄\x1b[0m  \x1b[34m蓝\x1b[0m  \x1b[35m洋红\x1b[0m  \x1b[36m青\x1b[0m\r\n");
            sb.Append("\x1b[38;5;196m256色-196\x1b[0m  \x1b[38;5;46m256色-46\x1b[0m  \x1b[48;5;236m 深底 \x1b[0m\r\n");
            sb.Append("\x1b[38;2;255;128;0m真彩-橙色\x1b[0m  \x1b[48;2;0;64;128m 深蓝底 \x1b[0m\r\n");
            sb.Append("\x1b[7m 反显 \x1b[0m  \x1b[4m 下划线 \x1b[0m  \x1b[9m 删除线 \x1b[0m\r\n");
            sb.Append("\r\n中文双宽：你好，世界！ 全角ＡＢＣ 半角ABC\r\n");
            sb.Append("┌──────────────┐\r\n│ 画的边框对不 │\r\n└──────────────┘\r\n");
            sb.Append("\x1b[13;1H\x1b[1;44;37m 状态栏：这是反显的一整行                        \x1b[0m");
            sb.Append("\x1b[15;3H光标定位测试 (15,3)");
            sb.Append("\x1b[17;1H超链接: \x1b]8;;https://example.com\x1b\\Ctrl+点击这里\x1b]8;;\x1b\\ 结束");
            sb.Append("\x1b[19;1H方向键测试: ABCD\x1b[3D  <- 光标应在 C 前");
            sb.Append("\x1b[18;1H演示结束，按「连接」可测试真实 SSH 会话");

            byte[] bytes = System.Text.Encoding.UTF8.GetBytes(sb.ToString());
            term.Feed(bytes, 0, bytes.Length);
            term.Resize(term.Rows, term.Cols);
        }

        // 显式布局，避开 WinForms Dock 的 z-order 坑
        protected override void OnLayout(LayoutEventArgs e)
        {
            base.OnLayout(e);
            if (view == null) return;   // 构造期间 Controls.Add 就会触发布局
            int w = ClientSize.Width;
            int h = ClientSize.Height;
            top.Bounds = new Rectangle(0, 0, w, 40);
            status.Bounds = new Rectangle(0, h - 22, w, 22);

            int searchHeight = searchPanel.Visible ? 36 : 0;
            searchPanel.Bounds = new Rectangle(0, 40, w, searchHeight);
            view.Bounds = new Rectangle(0, 40 + searchHeight, w, Math.Max(0, h - 62 - searchHeight));
        }

        private void ShowSearch()
        {
            searchPanel.Visible = true;
            PerformLayout();
            searchBox.Focus();
            searchBox.SelectAll();
        }

        private void HideSearch()
        {
            searchPanel.Visible = false;
            view.ClearSearch();
            view.Focus();
            PerformLayout();
        }

        private void UpdateSearchCount(int total)
        {
            if (total <= 0)
            {
                searchCount.Text = searchBox.Text.Length > 0 ? "无匹配" : "";
            }
            else
            {
                searchCount.Text = (view.SearchHitIndex + 1) + " / " + total;
            }
        }

        private void OnSearchKeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Escape)
            {
                HideSearch();
                e.Handled = true;
                e.SuppressKeyPress = true;
            }
            else if (e.KeyCode == Keys.Enter)
            {
                view.NextHit(!e.Shift);
                UpdateSearchCount(view.SearchHitCount);
                e.Handled = true;
                e.SuppressKeyPress = true;
            }
        }

        private void OnTitleChanged()
        {
            if (IsHandleCreated && !IsDisposed)
            {
                BeginInvoke((MethodInvoker)delegate
                {
                    Text = term.Title.Length > 0 ? term.Title + " - VT Spike" : "QShell Lite VT Spike";
                });
            }
        }

        /// <summary>远端用 OSC 52 请求写剪贴板（tmux copy-mode 等）。</summary>
        private void OnClipboardRequest(string text)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired)
            {
                BeginInvoke((MethodInvoker)delegate { OnClipboardRequest(text); });
                return;
            }
            try
            {
                Clipboard.SetText(text);
            }
            catch
            {
            }
        }

        private void OnConnect(object sender, EventArgs e)
        {
            if (shell != null)
            {
                Disconnect();
                connectButton.Text = "连接";
                return;
            }

            int port;
            if (!int.TryParse(portBox.Text, out port))
            {
                port = 22;
            }
            if (port <= 0) port = 22;

            try
            {
                status.Text = "连接中...";
                client = new SshClient(hostBox.Text.Trim(), port, userBox.Text.Trim(), pwdBox.Text);
                client.Connect();
                // 用当前视图的行列数创建 shell（列数要扣掉滚动条）
                int cols = Math.Max(10, view.ViewWidth / view.CellWidth);
                int rows = Math.Max(3, view.Height / view.CellHeight);
                term.Resize(rows, cols);
                shell = client.CreateShellStream("xterm-256color", (uint)cols, (uint)rows, 0, 0, 8192);
                channel = GetChannel(shell);
                channelResize = GetResizeMethod(channel);
                connectButton.Text = "断开";
                status.Text = "已连接 " + hostBox.Text;
                view.Focus();
            }
            catch (Exception ex)
            {
                status.Text = "连接失败: " + ex.Message;
                if (client != null)
                {
                    try { client.Dispose(); } catch { }
                    client = null;
                }
                shell = null;
            }
        }

        private void Disconnect()
        {
            try
            {
                if (shell != null)
                {
                    shell.Dispose();
                }
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
            status.Text = "已断开";
        }

        private void OnPump(object sender, EventArgs e)
        {
            ShellStream sh = shell;
            if (sh == null) return;
            try
            {
                var buf = new byte[8192];
                // 每次 tick 最多处理 256KB：大量输出时丢弃中间帧，保证界面不假死
                int budget = 262144;
                while (sh.DataAvailable && budget > 0)
                {
                    int n = sh.Read(buf, 0, Math.Min(buf.Length, budget));
                    if (n <= 0) break;
                    budget -= n;
                    if (rawDump != null)
                    {
                        try
                        {
                            rawDump.Write(buf, 0, n);
                            rawDump.Flush();     // 实时落盘，进程被强杀也不丢
                        }
                        catch { }
                    }
                    lastRx = (lastRx + ToHex(buf, n)).Trim();
                    if (lastRx.Length > 110)
                    {
                        lastRx = lastRx.Substring(lastRx.Length - 110);
                    }
                    UpdateDebugStatus();
                    term.Feed(buf, 0, n);
                }
            }
            catch
            {
            }
        }

        private string lastTx = "";
        private string lastRx = "";

        private void UpdateDebugStatus()
        {
            if (status == null || status.IsDisposed) return;
            status.Text = "TX: " + lastTx + "    RX: " + lastRx;
        }

        private static string ToHex(byte[] data, int count)
        {
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < count; i++)
            {
                sb.Append(data[i].ToString("X2")).Append(' ');
            }
            return sb.ToString();
        }

        private void OnTerminalInput(byte[] data)
        {
            if (txDump != null)
            {
                try
                {
                    txDump.Write(data, 0, data.Length);
                    txDump.Flush();
                }
                catch { }
            }

            // 把发出去的字节直接显示在状态栏，方便在没有日志的机器上排查按键
            lastTx = ToHex(data, data.Length);
            UpdateDebugStatus();

            ShellStream sh = shell;
            if (sh == null) return;
            try
            {
                sh.Write(data, 0, data.Length);
                sh.Flush();
            }
            catch (Exception ex)
            {
                status.Text = "发送失败: " + ex.Message;
            }
        }

        /// <summary>
        /// SSH.NET 2020 的 ShellStream 没有公开的窗口尺寸接口，
        /// 从它的私有 _channel 字段里拿到 channel（接口是 internal，只能用反射）。
        /// </summary>
        private static object GetChannel(ShellStream stream)
        {
            try
            {
                FieldInfo field = typeof(ShellStream).GetField(
                    "_channel", BindingFlags.NonPublic | BindingFlags.Instance);
                if (field != null)
                {
                    return field.GetValue(stream);
                }
            }
            catch
            {
            }
            return null;
        }

        private static MethodInfo GetResizeMethod(object channel)
        {
            if (channel == null)
            {
                return null;
            }
            try
            {
                return channel.GetType().GetMethod("SendWindowChangeRequest",
                    new Type[] { typeof(uint), typeof(uint), typeof(uint), typeof(uint) });
            }
            catch
            {
                return null;
            }
        }

        private void OnTerminalResized()
        {
            // 用 ViewWidth（扣掉滚动条），否则远端会多画一列、被滚动条遮住
            int cols = Math.Max(10, view.ViewWidth / view.CellWidth);
            int rows = Math.Max(3, view.Height / view.CellHeight);
            if (cols == term.Cols && rows == term.Rows) return;
            term.Resize(rows, cols);

            if (channelResize == null || channel == null) return;
            // 拖完再通知远端，避免拖动过程中反复重绘
            resizeTimer.Stop();
            resizeTimer.Start();
        }

        private void SendWindowResize()
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

        private void OnClosing(object sender, FormClosingEventArgs e)
        {
            pumpTimer.Stop();
            Disconnect();
            if (rawDump != null)
            {
                try { rawDump.Flush(); rawDump.Close(); } catch { }
                rawDump = null;
            }
            if (txDump != null)
            {
                try { txDump.Flush(); txDump.Close(); } catch { }
                txDump = null;
            }
        }
    }

    internal static class Program
    {
        [STAThread]
        private static void Main(string[] args)
        {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            bool demo = false;
            bool dump = true;      // 默认记录原始字节
            string replay = null;
            for (int i = 0; i < args.Length; i++)
            {
                if (args[i] == "--demo") demo = true;
                if (args[i] == "--dump") dump = true;
                if (args[i] == "--nodump") dump = false;
                if (args[i] == "--replay" && i + 1 < args.Length) { replay = args[i + 1]; i++; }
            }
            try
            {
                Application.Run(new SpikeForm(demo, dump, replay));
            }
            catch (Exception ex)
            {
                try
                {
                    System.IO.File.WriteAllText(
                        System.IO.Path.Combine(System.IO.Path.GetTempPath(), "vtspike-err.txt"),
                        ex.ToString());
                }
                catch { }
                throw;
            }
        }
    }
}
