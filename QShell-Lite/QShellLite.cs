// QShell Lite 原型（.NET Framework 4.0 / WinForms）
// 目的：验证老系统（Win7 / XP）上的观感、性能与"嵌入 PuTTY 作为终端"的可行性。
// 只做原型必需：读 QShell 的 config.json、列主机、双击启动 PuTTY 并嵌入窗口。
// 编译：csc /target:winexe /platform:x86 ...

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;
using System.Web.Script.Serialization;

namespace QShellLite
{
    internal sealed class HostEntry
    {
        public string Group;
        public string Name;
        public string Protocol;
        public string Host;
        public int Port;
        public string User;
        public string Pwd;
        public string KeyPath;

        public override string ToString()
        {
            return string.IsNullOrEmpty(Group) ? Name : Group + " / " + Name;
        }
    }

    internal sealed class LiteForm : Form
    {
        // ---- Win32（嵌入子窗口） ----
        [DllImport("user32.dll")]
        private static extern IntPtr SetParent(IntPtr child, IntPtr parent);
        [DllImport("user32.dll")]
        private static extern int GetWindowLong(IntPtr hWnd, int index);
        [DllImport("user32.dll")]
        private static extern int SetWindowLong(IntPtr hWnd, int index, int newStyle);
        [DllImport("user32.dll")]
        private static extern bool MoveWindow(IntPtr hWnd, int x, int y, int w, int h, bool repaint);
        [DllImport("user32.dll")]
        private static extern IntPtr SetFocus(IntPtr hWnd);
        [DllImport("user32.dll")]
        private static extern bool ShowWindow(IntPtr hWnd, int cmd);

        private const int GWL_STYLE = -16;
        private const int WS_CHILD = 0x40000000;
        private const int WS_VISIBLE = 0x10000000;
        private const int WS_CAPTION = 0x00C00000;
        private const int WS_THICKFRAME = 0x00040000;
        private const int WS_BORDER = 0x00800000;
        private const int WS_DLGFRAME = 0x00400000;
        private const int WS_SYSMENU = 0x00080000;
        private const int WS_POPUP = unchecked((int)0x80000000);
        private const int SW_SHOW = 5;

        private readonly ListBox hostList = new ListBox();
        private readonly Panel terminalPanel = new Panel();
        private readonly Label statusLabel = new Label();
        private readonly TextBox puttyBox = new TextBox();
        private readonly Button configButton = new Button();
        private readonly Button refreshButton = new Button();
        private readonly Button puttyButton = new Button();
        private readonly Button connectButton = new Button();

        private string configPath = "";
        private Process currentProcess;
        private IntPtr embeddedHandle = IntPtr.Zero;

        public LiteForm()
        {
            Text = "QShell Lite 原型（.NET 4.0 / WinForms）";
            StartPosition = FormStartPosition.CenterScreen;
            ClientSize = new Size(820, 540);
            MinimumSize = new Size(620, 400);
            Font = new Font("Segoe UI", 9f);
            try
            {
                // XP 上没有 Segoe UI，退回到系统默认字体
                if (Environment.OSVersion.Version.Major < 6) Font = SystemFonts.DefaultFont;
            }
            catch { }

            // ---------- 顶部工具条 ----------
            var topPanel = new Panel();
            topPanel.Dock = DockStyle.Top;
            topPanel.Height = 38;
            topPanel.Padding = new Padding(6, 6, 6, 4);
            Controls.Add(topPanel);

            configButton.Text = "选择配置...";
            configButton.Location = new Point(6, 6);
            configButton.Size = new Size(92, 26);
            configButton.Click += OnChooseConfig;
            topPanel.Controls.Add(configButton);

            refreshButton.Text = "刷新";
            refreshButton.Location = new Point(104, 6);
            refreshButton.Size = new Size(60, 26);
            refreshButton.Click += OnReloadConfig;
            topPanel.Controls.Add(refreshButton);

            connectButton.Text = "连接";
            connectButton.Location = new Point(170, 6);
            connectButton.Size = new Size(60, 26);
            connectButton.Click += OnConnectClick;
            topPanel.Controls.Add(connectButton);

            var puttyLabel = new Label();
            puttyLabel.Text = "PuTTY:";
            puttyLabel.AutoSize = true;
            puttyLabel.Location = new Point(246, 12);
            topPanel.Controls.Add(puttyLabel);

            puttyBox.Location = new Point(292, 8);
            puttyBox.Size = new Size(360, 24);
            topPanel.Controls.Add(puttyBox);

            puttyButton.Text = "浏览...";
            puttyButton.Size = new Size(64, 26);
            puttyButton.Location = new Point(658, 6);
            puttyButton.Click += OnChoosePutty;
            topPanel.Controls.Add(puttyButton);

            // 工具条宽度变化时重排：原来用 Anchor 会在初始化阶段算错位置，
            // 导致路径框盖住「浏览...」按钮（Win7 截图里就是这样）。
            topPanel.Layout += delegate
            {
                int width = topPanel.ClientSize.Width;
                puttyButton.Left = width - puttyButton.Width - 8;
                puttyBox.Left = 292;
                puttyBox.Width = Math.Max(80, puttyButton.Left - puttyBox.Left - 8);
            };

            // ---------- 状态栏（先加，保证能 Dock 到底部） ----------
            var statusPanel = new Panel();
            statusPanel.Dock = DockStyle.Bottom;
            statusPanel.Height = 24;
            statusPanel.Padding = new Padding(8, 4, 8, 0);
            Controls.Add(statusPanel);

            statusLabel.Dock = DockStyle.Fill;
            statusLabel.TextAlign = ContentAlignment.MiddleLeft;
            statusLabel.Text = "就绪";
            statusPanel.Controls.Add(statusLabel);

            // ---------- 左侧主机列表 ----------
            hostList.Dock = DockStyle.Left;
            hostList.Width = 220;
            hostList.BorderStyle = BorderStyle.FixedSingle;
            hostList.IntegralHeight = false;
            hostList.DoubleClick += OnHostDoubleClick;
            Controls.Add(hostList);

            // ---------- 右侧终端区 ----------
            terminalPanel.Dock = DockStyle.Fill;
            terminalPanel.BorderStyle = BorderStyle.FixedSingle;
            terminalPanel.BackColor = Color.Black;
            terminalPanel.Resize += delegate { ResizeEmbedded(); };
            Controls.Add(terminalPanel);
            terminalPanel.BringToFront();

            // 默认路径猜测
            string exeDir = Path.GetDirectoryName(Application.ExecutablePath);
            string guessPutty = Path.Combine(exeDir, "putty.exe");
            puttyBox.Text = guessPutty;

            string guessConfig = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                Path.Combine("QShell", "config.json"));
            if (File.Exists(guessConfig))
            {
                configPath = guessConfig;
                LoadConfig();
            }
            else
            {
                LoadSamples();
                SetStatus("未找到 QShell 配置（当前显示示例数据），请点「选择配置...」指定 config.json");
            }

            FormClosing += OnFormClosing;
        }

        /// <summary>没配置文件时放几条示例，方便先看界面与交互。</summary>
        private void LoadSamples()
        {
            var list = new[]
            {
                new HostEntry { Group = "示例分组", Name = "演示主机 1", Protocol = "ssh", Host = "192.168.1.10", Port = 22, User = "root" },
                new HostEntry { Group = "示例分组", Name = "演示主机 2", Protocol = "ssh", Host = "192.168.1.11", Port = 22, User = "admin" },
                new HostEntry { Group = "示例分组", Name = "OpenWrt 路由", Protocol = "ssh", Host = "192.168.1.1", Port = 22, User = "root" }
            };

            hostList.BeginUpdate();
            hostList.Items.Clear();
            foreach (HostEntry h in list)
            {
                hostList.Items.Add(h);
            }
            hostList.EndUpdate();
        }

        // ---------------- 配置 ----------------

        private void OnChooseConfig(object sender, EventArgs e)
        {
            var dlg = new OpenFileDialog();
            dlg.Filter = "QShell 配置 (*.json)|*.json|所有文件 (*.*)|*.*";
            dlg.Title = "选择 QShell 的 config.json";
            if (dlg.ShowDialog(this) == DialogResult.OK)
            {
                configPath = dlg.FileName;
                LoadConfig();
            }
        }

        private void OnReloadConfig(object sender, EventArgs e)
        {
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("还没有选择配置文件");
                return;
            }
            LoadConfig();
        }

        private void LoadConfig()
        {
            try
            {
                string json = File.ReadAllText(configPath, Encoding.UTF8);
                List<HostEntry> hosts = ParseHosts(json);

                hostList.BeginUpdate();
                hostList.Items.Clear();
                foreach (HostEntry h in hosts)
                {
                    hostList.Items.Add(h);
                }
                hostList.EndUpdate();

                SetStatus(string.Format("已载入 {0} 台主机 —— {1}", hosts.Count, configPath));
            }
            catch (Exception ex)
            {
                SetStatus("读取配置失败: " + ex.Message);
            }
        }

        private static List<HostEntry> ParseHosts(string json)
        {
            var result = new List<HostEntry>();
            var serializer = new JavaScriptSerializer();
            serializer.MaxJsonLength = int.MaxValue;

            var root = serializer.Deserialize<Dictionary<string, object>>(json);
            if (root == null)
            {
                return result;
            }

            object devicesObj;
            if (!root.TryGetValue("devices", out devicesObj))
            {
                return result;
            }

            var devices = devicesObj as object[];
            if (devices == null)
            {
                return result;
            }

            foreach (object deviceObj in devices)
            {
                var device = deviceObj as Dictionary<string, object>;
                if (device == null)
                {
                    continue;
                }

                string group = AsString(device, "group");
                object hostsObj;
                if (!device.TryGetValue("hosts", out hostsObj))
                {
                    continue;
                }

                var hosts = hostsObj as object[];
                if (hosts == null)
                {
                    continue;
                }

                foreach (object hostObj in hosts)
                {
                    var h = hostObj as Dictionary<string, object>;
                    if (h == null)
                    {
                        continue;
                    }

                    var entry = new HostEntry();
                    entry.Group = group;
                    entry.Name = AsString(h, "name");
                    entry.Protocol = AsString(h, "protocol");
                    entry.Host = AsString(h, "host");
                    entry.Port = AsInt(h, "port", 22);
                    entry.User = AsString(h, "user");
                    entry.Pwd = AsString(h, "pwd");
                    entry.KeyPath = AsString(h, "key_path");
                    if (string.IsNullOrEmpty(entry.Name))
                    {
                        entry.Name = entry.Host;
                    }
                    result.Add(entry);
                }
            }

            return result;
        }

        private static string AsString(Dictionary<string, object> map, string key)
        {
            object value;
            if (map.TryGetValue(key, out value) && value != null)
            {
                return Convert.ToString(value);
            }
            return "";
        }

        private static int AsInt(Dictionary<string, object> map, string key, int fallback)
        {
            object value;
            if (map.TryGetValue(key, out value) && value != null)
            {
                int parsed;
                if (int.TryParse(Convert.ToString(value), out parsed))
                {
                    return parsed;
                }
            }
            return fallback;
        }

        // ---------------- PuTTY ----------------

        private void OnChoosePutty(object sender, EventArgs e)
        {
            var dlg = new OpenFileDialog();
            dlg.Filter = "PuTTY (putty.exe)|putty.exe|可执行文件 (*.exe)|*.exe";
            dlg.Title = "选择 putty.exe";
            if (dlg.ShowDialog(this) == DialogResult.OK)
            {
                puttyBox.Text = dlg.FileName;
            }
        }

        private void OnHostDoubleClick(object sender, EventArgs e)
        {
            ConnectSelected();
        }

        private void OnConnectClick(object sender, EventArgs e)
        {
            ConnectSelected();
        }

        private void ConnectSelected()
        {
            var entry = hostList.SelectedItem as HostEntry;
            if (entry == null)
            {
                SetStatus("请先在左侧选择一台主机");
                return;
            }

            string putty = puttyBox.Text.Trim();
            if (putty.Length == 0 || !File.Exists(putty))
            {
                SetStatus("找不到 putty.exe，请点「浏览...」指定");
                return;
            }

            if (currentProcess != null && !currentProcess.HasExited)
            {
                SetStatus("原型一次只跑一个会话，请先关闭当前会话");
                return;
            }

            var psi = new ProcessStartInfo(putty, BuildArgs(entry));
            psi.UseShellExecute = false;
            psi.WorkingDirectory = Path.GetDirectoryName(putty);

            try
            {
                currentProcess = Process.Start(psi);
            }
            catch (Exception ex)
            {
                SetStatus("启动 PuTTY 失败: " + ex.Message);
                return;
            }

            if (currentProcess == null)
            {
                SetStatus("启动 PuTTY 失败");
                return;
            }

            SetStatus("正在等待 PuTTY 窗口...");
            IntPtr hwnd = WaitForWindow(currentProcess, 12000);
            if (hwnd == IntPtr.Zero)
            {
                SetStatus("等待 PuTTY 窗口超时（可尝试把 putty.exe 放到本程序同目录）");
                return;
            }

            Embed(hwnd);
            SetStatus(string.Format("已嵌入会话: {0}  ({1}@{2}:{3})", entry.Name, entry.User, entry.Host, entry.Port));
        }

        private static string BuildArgs(HostEntry entry)
        {
            var sb = new StringBuilder();

            string protocol = entry.Protocol == null ? "" : entry.Protocol.Trim().ToLowerInvariant();
            if (protocol == "telnet")
            {
                sb.Append("-telnet ");
            }
            else if (protocol == "raw")
            {
                sb.Append("-raw ");
            }
            else if (protocol == "rlogin")
            {
                sb.Append("-rlogin ");
            }
            else
            {
                sb.Append("-ssh ");
            }

            if (!string.IsNullOrEmpty(entry.User))
            {
                sb.Append(entry.User).Append('@');
            }
            sb.Append(entry.Host);
            sb.Append(" -P ").Append(entry.Port);

            if (!string.IsNullOrEmpty(entry.Pwd))
            {
                sb.Append(" -pw \"").Append(entry.Pwd.Replace("\"", "")).Append('"');
            }
            else if (!string.IsNullOrEmpty(entry.KeyPath))
            {
                sb.Append(" -i \"").Append(entry.KeyPath).Append('"');
            }

            return sb.ToString();
        }

        private static IntPtr WaitForWindow(Process process, int timeoutMs)
        {
            int waited = 0;
            while (waited < timeoutMs)
            {
                try
                {
                    if (process.HasExited)
                    {
                        return IntPtr.Zero;
                    }
                    process.Refresh();
                    if (process.MainWindowHandle != IntPtr.Zero)
                    {
                        return process.MainWindowHandle;
                    }
                }
                catch
                {
                    return IntPtr.Zero;
                }

                Thread.Sleep(120);
                waited += 120;
                Application.DoEvents();
            }
            return IntPtr.Zero;
        }

        private void Embed(IntPtr hwnd)
        {
            embeddedHandle = hwnd;
            SetParent(hwnd, terminalPanel.Handle);

            int style = GetWindowLong(hwnd, GWL_STYLE);
            style &= ~(WS_CAPTION | WS_THICKFRAME | WS_BORDER | WS_DLGFRAME | WS_SYSMENU | WS_POPUP);
            style |= WS_CHILD | WS_VISIBLE;
            SetWindowLong(hwnd, GWL_STYLE, style);

            ResizeEmbedded();
            ShowWindow(hwnd, SW_SHOW);
            SetFocus(hwnd);
        }

        private void ResizeEmbedded()
        {
            if (embeddedHandle == IntPtr.Zero)
            {
                return;
            }
            MoveWindow(embeddedHandle, 0, 0, terminalPanel.ClientSize.Width, terminalPanel.ClientSize.Height, true);
        }

        private void SetStatus(string text)
        {
            statusLabel.Text = text;
        }

        private void OnFormClosing(object sender, FormClosingEventArgs e)
        {
            try
            {
                if (currentProcess != null && !currentProcess.HasExited)
                {
                    currentProcess.Kill();
                }
            }
            catch { }
        }
    }

    internal static class Program
    {
        [STAThread]
        private static void Main()
        {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new LiteForm());
        }
    }
}
