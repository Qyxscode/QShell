// 主窗口：工具条 + 主机列表 + 多标签终端 + 状态栏。
// 每个标签页 = 一个 SshSession（SSH 连接 + 终端控件）。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class LiteForm : Form
    {
        private const string AppTitle = "QShell Lite 0.3";

        private const int ToolbarHeight = 38;
        private const int StatusHeight = 24;
        private const int HostListWidth = 220;

        private readonly TreeView hostTree = new TreeView();
        private readonly MemoPanel memoPanel = new MemoPanel();
        private const int MemoPanelWidth = 220;
        private readonly TabControl tabs = new TabControl();

        private readonly Panel welcomePanel = new Panel();
        private readonly Label welcomeLabel = new Label();
        private readonly Panel topPanel = new Panel();
        private readonly Panel statusPanel = new Panel();
        private readonly Label statusLabel = new Label();
        private readonly Label sessionCountLabel = new Label();
        private readonly ContextMenuStrip tabMenu = new ContextMenuStrip();
        private readonly List<ISession> sessions = new List<ISession>();

        private Button configButton;
        private Button refreshButton;
        private Button connectButton;
        private Button disconnectButton;

        private string configPath = "";
        private TreeNode contextNode;   // 右键时鼠标下的节点
        private bool suppressBodyRefresh;

        // 外观（来自 config.json 的 settings，也可在设置窗口改）
        private string appearanceFont = "";
        private float appearanceSize = 10f;
        private bool appearanceBold;
        private string appearanceCursor = "block";
        private bool appearanceBlink = true;
        private int appearanceFg = Color.FromArgb(0xCC, 0xCC, 0xCC).ToArgb();
        private bool autoReconnect;
        private bool sessionLogEnabled;
        private bool layingOut;

        public LiteForm()
            : this(null)
        {
        }

        public LiteForm(string initialConfigPath)
        {
            Text = AppTitle;
            StartPosition = FormStartPosition.CenterScreen;
            ClientSize = new Size(1240, 690);
            MinimumSize = new Size(720, 460);
            ApplyFont();

            // ---------- 顶部工具条 ----------
            topPanel.Padding = new Padding(6, 6, 6, 4);
            Controls.Add(topPanel);

            configButton = AddToolButton(topPanel, "选择配置...", 6, 96, OnChooseConfig);
            refreshButton = AddToolButton(topPanel, "刷新", 108, 56, OnReloadConfig);
            connectButton = AddToolButton(topPanel, "连接", 170, 56, OnConnectClick);
            disconnectButton = AddToolButton(topPanel, "断开", 232, 56, OnDisconnectClick);
            AddToolButton(topPanel, "外观设置", 294, 76, OnOpenAppearance);
            AddToolButton(topPanel, "设置", 376, 56, OnOpenSettings);

            var hint = new Label();
            hint.AutoSize = true;
            hint.ForeColor = SystemColors.GrayText;
            hint.Location = new Point(440, 12);
            hint.Text = "双击主机或点「连接」开始会话";
            topPanel.Controls.Add(hint);

            // ---------- 状态栏 ----------
            statusPanel.Padding = new Padding(8, 4, 8, 0);
            Controls.Add(statusPanel);

            sessionCountLabel.AutoSize = false;
            sessionCountLabel.Size = new Size(110, 18);
            sessionCountLabel.TextAlign = ContentAlignment.MiddleRight;
            sessionCountLabel.Text = "无会话";
            statusPanel.Controls.Add(sessionCountLabel);

            statusLabel.Dock = DockStyle.Fill;
            statusLabel.TextAlign = ContentAlignment.MiddleLeft;
            statusLabel.Padding = new Padding(0, 0, 120, 0);
            statusLabel.Text = "就绪";
            statusPanel.Controls.Add(statusLabel);

            statusPanel.Layout += delegate
            {
                sessionCountLabel.Left = Math.Max(0, statusPanel.ClientSize.Width - sessionCountLabel.Width - 8);
                sessionCountLabel.Top = 3;
            };

            // ---------- 左侧主机列表 ----------
            hostTree.BorderStyle = BorderStyle.FixedSingle;
            hostTree.HideSelection = false;
            hostTree.FullRowSelect = true;
            hostTree.ShowLines = true;
            hostTree.ShowPlusMinus = true;
            hostTree.ShowRootLines = true;
            hostTree.AllowDrop = true;
            hostTree.ItemDrag += delegate(object s, ItemDragEventArgs e)
            {
                if (e.Item is TreeNode) hostTree.DoDragDrop(e.Item, DragDropEffects.Move);
            };
            hostTree.DragOver += delegate(object s, DragEventArgs e)
            {
                e.Effect = DragDropEffects.Move;
            };
            hostTree.DragDrop += OnHostTreeDragDrop;
            hostTree.NodeMouseDoubleClick += OnHostNodeDoubleClick;
            hostTree.AfterSelect += delegate { UpdateChrome(); };
            // 右键先选中节点，否则菜单操作会作用到上一次选中的主机上
            hostTree.MouseDown += delegate(object s, MouseEventArgs e)
            {
                if (e.Button == MouseButtons.Right)
                {
                    TreeNode node = hostTree.GetNodeAt(e.X, e.Y);
                    if (node != null) hostTree.SelectedNode = node;
                }
            };
            Controls.Add(hostTree);

            // 右键菜单按目标重建（空白 / 分组 / 主机），避免隐藏项残影
            var hostMenu = new ContextMenuStrip();
            hostMenu.Opening += delegate
            {
                Point pt = hostTree.PointToClient(Cursor.Position);
                contextNode = hostTree.GetNodeAt(pt);
                hostMenu.Items.Clear();

                bool isHost = contextNode != null && contextNode.Tag is HostEntry;
                bool isGroup = contextNode != null && contextNode.Tag is string;

                if (isHost)
                {
                    hostMenu.Items.Add("编辑主机...", null, OnHostMenuEdit);
                    hostMenu.Items.Add("克隆主机", null, OnHostMenuDuplicate);
                    hostMenu.Items.Add("复制主机地址", null, OnHostMenuCopyAddress);
                    hostMenu.Items.Add("测试延迟", null, OnHostMenuProbeLatency);
                    hostMenu.Items.Add(new ToolStripSeparator());
                    hostMenu.Items.Add("删除主机", null, OnHostMenuDelete);
                }
                else if (isGroup)
                {
                    hostMenu.Items.Add("在此组新建主机...", null, OnHostMenuAdd);
                    hostMenu.Items.Add(new ToolStripSeparator());
                    hostMenu.Items.Add("重命名分组...", null, OnHostMenuRenameGroup);
                    hostMenu.Items.Add("删除分组", null, OnHostMenuDeleteGroup);
                }
                else
                {
                    hostMenu.Items.Add("新建主机...", null, OnHostMenuAdd);
                    hostMenu.Items.Add("新建分组...", null, OnHostMenuNewGroup);
                }
            };
            hostTree.ContextMenuStrip = hostMenu;

            // ---------- 右侧备忘录面板 ----------
            memoPanel.RunCommand += OnRunMemoCommand;
            memoPanel.Changed += OnMemoChanged;
            Controls.Add(memoPanel);

            // ---------- 右侧会话区（多标签） ----------
            tabs.Padding = new Point(14, 4);
            tabs.Visible = false;
            // 原生外观 + 每个标签一个 × 图标当关闭按钮（图标在文字左侧）
            tabs.ImageList = CreateTabIcons();
            tabs.MouseDown += OnTabsMouseDown;
            tabs.SelectedIndexChanged += OnTabSelected;
            tabs.MouseUp += OnTabsMouseUp;
            Controls.Add(tabs);

            welcomePanel.BackColor = Color.Black;
            Controls.Add(welcomePanel);

            welcomeLabel.Dock = DockStyle.Fill;
            welcomeLabel.BackColor = Color.Black;
            welcomeLabel.ForeColor = Color.FromArgb(150, 150, 150);
            welcomeLabel.TextAlign = ContentAlignment.MiddleCenter;
            welcomeLabel.Text = "双击左侧主机开始连接\r\n（每个会话一个标签页）";
            welcomePanel.Controls.Add(welcomeLabel);

            // ---------- 标签页右键菜单 ----------
            tabMenu.Items.Add("关闭此会话", null, OnMenuCloseCurrent);
            tabMenu.Items.Add("关闭其它会话", null, OnMenuCloseOthers);
            tabMenu.Items.Add("全部关闭", null, OnMenuCloseAll);

            // ---------- 配置 ----------
            string defaultConfig = ConfigStore.DefaultPath;
            if (!string.IsNullOrEmpty(initialConfigPath))
            {
                configPath = initialConfigPath;
                LoadConfig();
            }
            else if (File.Exists(defaultConfig))
            {
                configPath = defaultConfig;
                LoadConfig();
            }
            else
            {
                FillHostTree(ConfigStore.Samples(), null);
                SetStatus("未找到 QShell 配置（当前显示示例数据），请点「选择配置...」指定 config.json");
            }

            FormClosing += OnFormClosing;
            UpdateChrome();

            // 小屏幕（如 1024x768）上别超出屏幕
            try
            {
                Rectangle wa = Screen.PrimaryScreen.WorkingArea;
                if (ClientSize.Width > wa.Width - 60 || ClientSize.Height > wa.Height - 60)
                {
                    ClientSize = new Size(
                        Math.Min(ClientSize.Width, Math.Max(720, wa.Width - 60)),
                        Math.Min(ClientSize.Height, Math.Max(460, wa.Height - 60)));
                }
            }
            catch
            {
            }
        }

        private void ApplyFont()
        {
            try
            {
                // XP 上没有 Segoe UI，用系统默认字体
                if (Environment.OSVersion.Version.Major >= 6)
                {
                    Font = new Font("Segoe UI", 9f);
                }
                else
                {
                    Font = SystemFonts.DefaultFont;
                }
            }
            catch
            {
            }
        }

        // ---------------- 布局 ----------------

        protected override void OnLayout(LayoutEventArgs levent)
        {
            base.OnLayout(levent);
            ApplyLayout();
        }

        /// <summary>
        /// 显式排出四个区：工具条（顶）、状态栏（底）、主机列表（左）、会话区（其余）。
        /// 不用 Dock：Dock 依赖 Controls 的 z-order，实测会让左侧列表盖住工具条左端。
        /// </summary>
        private void ApplyLayout()
        {
            if (layingOut)
            {
                return;
            }
            layingOut = true;
            try
            {
                int width = ClientSize.Width;
                int height = ClientSize.Height;
                if (width <= 0 || height <= 0)
                {
                    return;
                }

                int bodyHeight = Math.Max(0, height - ToolbarHeight - StatusHeight);

                topPanel.Bounds = new Rectangle(0, 0, width, ToolbarHeight);
                statusPanel.Bounds = new Rectangle(0, height - StatusHeight, width, StatusHeight);
                hostTree.Bounds = new Rectangle(0, ToolbarHeight, HostListWidth, bodyHeight);
                memoPanel.Bounds = new Rectangle(width - MemoPanelWidth, ToolbarHeight, MemoPanelWidth, bodyHeight);

                var body = new Rectangle(
                    HostListWidth,
                    ToolbarHeight,
                    Math.Max(0, width - HostListWidth - MemoPanelWidth),
                    bodyHeight);
                tabs.Bounds = body;
                welcomePanel.Bounds = body;
            }
            finally
            {
                layingOut = false;
            }
        }

        private static Button AddToolButton(Panel parent, string text, int left, int width, EventHandler onClick)
        {
            var button = new Button();
            button.Text = text;
            button.Location = new Point(left, 6);
            button.Size = new Size(width, 26);
            if (onClick != null)
            {
                button.Click += onClick;
            }
            parent.Controls.Add(button);
            return button;
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
                List<HostEntry> hosts = ConfigStore.Load(configPath);
                FillHostTree(hosts, ConfigStore.LoadGroups(configPath));
                LoadMemosIntoPanel();
                LoadAppearance();
                SetStatus(string.Format("已载入 {0} 台主机 —— {1}", hosts.Count, configPath));
            }
            catch (Exception ex)
            {
                SetStatus("读取配置失败: " + ex.Message);
            }
        }

        /// <summary>按分组填充成树：分组是父节点，主机是子节点，可折叠。</summary>
        private void FillHostTree(List<HostEntry> hosts, List<string> existingGroups)
        {
            hostTree.BeginUpdate();
            try
            {
                hostTree.Nodes.Clear();
                var groups = new List<string>();
                // 先放配置文件里存在的分组（包括还没有主机的空分组）
                if (existingGroups != null)
                {
                    foreach (string g in existingGroups)
                    {
                        if (!groups.Contains(g)) groups.Add(g);
                    }
                }
                foreach (HostEntry h in hosts)
                {
                    if (!groups.Contains(h.Group))
                    {
                        groups.Add(h.Group);
                    }
                }

                foreach (string group in groups)
                {
                    int count = 0;
                    foreach (HostEntry h in hosts)
                    {
                        if (h.Group == group) count++;
                    }

                    string label = group.Length > 0 ? group : "(未分组)";
                    TreeNode node = hostTree.Nodes.Add(label + "  (" + count + ")");
                    node.Tag = group;

                    foreach (HostEntry h in hosts)
                    {
                        if (h.Group != group) continue;
                        string hostLabel = h.DisplayName;
                        if (h.LatencyMs >= 0) hostLabel += "   [" + h.LatencyMs + "ms]";
                        TreeNode child = node.Nodes.Add(hostLabel);
                        child.Tag = h;
                        child.ToolTipText = h.User + "@" + h.Host + ":" + h.Port;
                    }
                }
                hostTree.ExpandAll();
            }
            finally
            {
                hostTree.EndUpdate();
            }
        }

        /// <summary>当前选中的主机（选在分组上时为 null）。</summary>
        private HostEntry SelectedHost
        {
            get
            {
                if (hostTree.SelectedNode == null) return null;
                return hostTree.SelectedNode.Tag as HostEntry;
            }
        }

        /// <summary>会话状态消息（断线/重连）转到状态栏。</summary>
        private void OnStatusMessage(string text)
        {
            if (IsDisposed || !IsHandleCreated) return;
            if (InvokeRequired)
            {
                BeginInvoke((MethodInvoker)delegate { OnStatusMessage(text); });
                return;
            }
            SetStatus(text);
        }

        // ---------------- 外观设置 ----------------

        private void LoadAppearance()
        {
            try
            {
                Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                Dictionary<string, object> settings = ConfigStore.LoadSettings(root);

                appearanceFont = SettingString(settings, "font_family");
                appearanceSize = SettingFloat(settings, "font_size", 10f);
                appearanceBold = SettingBool(settings, "is_bold", false);
                appearanceCursor = SettingString(settings, "cursor_style");
                if (appearanceCursor.Length == 0) appearanceCursor = "block";
                appearanceBlink = SettingBool(settings, "cursor_blink", true);
                appearanceFg = ParseColor(SettingString(settings, "terminal_fg_color"),
                    Color.FromArgb(0xCC, 0xCC, 0xCC).ToArgb());
                autoReconnect = SettingBool(settings, "auto_reconnect", false);
                sessionLogEnabled = SettingBool(settings, "session_log_enabled", false);

                foreach (ISession s in sessions)
                {
                    ApplyAppearance(s);
                }
            }
            catch
            {
            }
        }

        private static string SettingString(Dictionary<string, object> map, string key)
        {
            object v;
            if (map != null && map.TryGetValue(key, out v) && v != null) return Convert.ToString(v);
            return "";
        }

        private static float SettingFloat(Dictionary<string, object> map, string key, float fallback)
        {
            string text = SettingString(map, key);
            float parsed;
            if (float.TryParse(text, System.Globalization.NumberStyles.Float,
                System.Globalization.CultureInfo.InvariantCulture, out parsed))
            {
                return parsed;
            }
            return fallback;
        }

        private static bool SettingBool(Dictionary<string, object> map, string key, bool fallback)
        {
            object v;
            if (map != null && map.TryGetValue(key, out v) && v != null)
            {
                if (v is bool) return (bool)v;
                string text = Convert.ToString(v).ToLowerInvariant();
                if (text == "true") return true;
                if (text == "false") return false;
            }
            return fallback;
        }

        private static int ParseColor(string text, int fallback)
        {
            if (string.IsNullOrEmpty(text)) return fallback;
            try
            {
                string hex = text.Trim().TrimStart('#');
                if (hex.Length == 6)
                {
                    int r = Convert.ToInt32(hex.Substring(0, 2), 16);
                    int g = Convert.ToInt32(hex.Substring(2, 2), 16);
                    int b = Convert.ToInt32(hex.Substring(4, 2), 16);
                    return Color.FromArgb(r, g, b).ToArgb();
                }
            }
            catch
            {
            }
            return fallback;
        }

        private void ApplyAppearance(ISession session)
        {
            if (session == null) return;
            ApplyAppearanceToView(session.View);
        }

        private void ApplyAppearanceToView(TerminalView view)
        {
            if (view == null) return;
            view.ApplyAppearance(appearanceFont, appearanceSize, appearanceBold,
                appearanceFg, Color.FromArgb(0x0A, 0x0B, 0x0E).ToArgb(),
                Color.FromArgb(220, 220, 220, 220), appearanceCursor, appearanceBlink);
        }

        private void OnOpenAppearance(object sender, EventArgs e)
        {
            var dlg = new SettingsDialog(appearanceFont, appearanceSize, appearanceBold,
                appearanceCursor, appearanceBlink, appearanceFg);
            if (dlg.ShowDialog(this) != DialogResult.OK) return;

            appearanceFont = dlg.FontFamilyValue ?? "";
            appearanceSize = dlg.FontSizeValue;
            appearanceBold = dlg.BoldValue;
            appearanceCursor = dlg.CursorStyleValue;
            appearanceBlink = dlg.CursorBlinkValue;
            appearanceFg = dlg.ForeColorValue;

            foreach (ISession s in sessions)
            {
                ApplyAppearance(s);
            }

            if (!string.IsNullOrEmpty(configPath))
            {
                try
                {
                    Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                    Dictionary<string, object> settings = ConfigStore.LoadSettings(root);
                    settings["font_family"] = appearanceFont;
                    settings["font_size"] = (int)appearanceSize;
                    settings["is_bold"] = appearanceBold;
                    settings["cursor_style"] = appearanceCursor;
                    settings["cursor_blink"] = appearanceBlink;
                    Color fg = Color.FromArgb(appearanceFg);
                    settings["terminal_fg_color"] = string.Format("#{0:X2}{1:X2}{2:X2}", fg.R, fg.G, fg.B);
                    ConfigStore.SaveSettings(root, settings);
                    ConfigStore.WriteRoot(configPath, root);
                }
                catch (Exception ex)
                {
                    SetStatus("保存设置失败: " + ex.Message);
                    return;
                }
            }
            SetStatus("外观已应用");
        }

        private void OnOpenSettings(object sender, EventArgs e)
        {
            var dlg = new AppSettingsDialog(autoReconnect, sessionLogEnabled);
            if (dlg.ShowDialog(this) != DialogResult.OK) return;

            autoReconnect = dlg.AutoReconnectValue;
            sessionLogEnabled = dlg.SessionLogValue;

            if (dlg.RequestExport)
            {
                OnExportBackup(null, null);
                return;
            }
            if (dlg.RequestImport)
            {
                OnImportBackup(null, null);
                return;
            }

            // 立即作用到已经开着的会话
            foreach (ISession s in sessions)
            {
                s.AutoReconnect = autoReconnect;
                s.LogEnabled = sessionLogEnabled;
            }

            if (!string.IsNullOrEmpty(configPath))
            {
                try
                {
                    Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                    Dictionary<string, object> settings = ConfigStore.LoadSettings(root);
                    settings["auto_reconnect"] = autoReconnect;
                    settings["session_log_enabled"] = sessionLogEnabled;
                    ConfigStore.SaveSettings(root, settings);
                    ConfigStore.WriteRoot(configPath, root);
                }
                catch (Exception ex)
                {
                    SetStatus("保存设置失败: " + ex.Message);
                    return;
                }
            }
            SetStatus("设置已保存");
        }

        // ---------------- 导入 / 导出 ----------------

        /// <summary>导出配置：.zip 里放 qshell_config.json，或用 .json 直接写。</summary>
        private void OnExportBackup(object sender, EventArgs e)
        {
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("没有配置文件可导出");
                return;
            }
            var dlg = new SaveFileDialog();
            dlg.Title = "导出配置备份";
            dlg.Filter = "QShell 备份 (*.zip)|*.zip|JSON 文件 (*.json)|*.json";
            dlg.FileName = "qshell_backup_" + DateTime.Now.ToString("yyyyMMdd_HHmm") + ".zip";
            if (dlg.ShowDialog(this) != DialogResult.OK) return;

            try
            {
                string json = System.IO.File.ReadAllText(configPath, System.Text.Encoding.UTF8);
                // 先确认是合法 JSON
                var check = new System.Web.Script.Serialization.JavaScriptSerializer();
                check.Deserialize<Dictionary<string, object>>(json);

                if (dlg.FileName.ToLowerInvariant().EndsWith(".zip"))
                {
                    WriteZip(dlg.FileName, "qshell_config.json", json);
                }
                else
                {
                    System.IO.File.WriteAllText(dlg.FileName, json, new System.Text.UTF8Encoding(false));
                }
                SetStatus("已导出到 " + dlg.FileName);
            }
            catch (Exception ex)
            {
                SetStatus("导出失败: " + ex.Message);
            }
        }

        /// <summary>导入配置：从 .zip（qshell_config.json / config.json）或 .json 读，覆盖本地配置。</summary>
        private void OnImportBackup(object sender, EventArgs e)
        {
            // 没有配置路径就当"新建配置"：导入后落到默认位置
            if (string.IsNullOrEmpty(configPath))
            {
                configPath = ConfigStore.DefaultPath;
                try
                {
                    string dir = System.IO.Path.GetDirectoryName(configPath);
                    if (!string.IsNullOrEmpty(dir)) System.IO.Directory.CreateDirectory(dir);
                }
                catch { }
            }
            var dlg = new OpenFileDialog();
            dlg.Title = "导入配置备份";
            dlg.Filter = "备份文件 (*.zip;*.json)|*.zip;*.json|所有文件 (*.*)|*.*";
            if (dlg.ShowDialog(this) != DialogResult.OK) return;

            string json;
            try
            {
                if (dlg.FileName.ToLowerInvariant().EndsWith(".zip"))
                {
                    json = ReadZip(dlg.FileName, "qshell_config.json");
                    if (json == null) json = ReadZip(dlg.FileName, "config.json");
                    if (json == null)
                    {
                        SetStatus("压缩包里没找到 qshell_config.json 或 config.json");
                        return;
                    }
                }
                else
                {
                    json = System.IO.File.ReadAllText(dlg.FileName, System.Text.Encoding.UTF8);
                }

                var check = new System.Web.Script.Serialization.JavaScriptSerializer();
                check.Deserialize<Dictionary<string, object>>(json);
            }
            catch (Exception ex)
            {
                SetStatus("备份文件无效: " + ex.Message);
                return;
            }

            if (MessageBox.Show(this,
                "导入会覆盖当前配置（原文件会备份成 .bak）。继续？", "导入确认",
                MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes)
            {
                return;
            }

            try
            {
                if (System.IO.File.Exists(configPath))
                {
                    System.IO.File.Copy(configPath, configPath + ".bak", true);
                }
                System.IO.File.WriteAllText(configPath, json, new System.Text.UTF8Encoding(false));
                LoadConfig();
                SetStatus("已导入配置");
            }
            catch (Exception ex)
            {
                SetStatus("导入失败: " + ex.Message);
            }
        }

        private static void WriteZip(string path, string entryName, string content)
        {
            using (var fs = new System.IO.FileStream(path, System.IO.FileMode.Create))
            using (var zip = new System.IO.Compression.ZipArchive(fs,
                System.IO.Compression.ZipArchiveMode.Create))
            {
                var entry = zip.CreateEntry(entryName,
                    System.IO.Compression.CompressionLevel.Optimal);
                using (var stream = entry.Open())
                using (var writer = new System.IO.StreamWriter(stream, new System.Text.UTF8Encoding(false)))
                {
                    writer.Write(content);
                }
            }
        }

        private static string ReadZip(string path, string entryName)
        {
            using (var fs = new System.IO.FileStream(path, System.IO.FileMode.Open,
                System.IO.FileAccess.Read))
            using (var zip = new System.IO.Compression.ZipArchive(fs,
                System.IO.Compression.ZipArchiveMode.Read))
            {
                foreach (var entry in zip.Entries)
                {
                    if (string.Equals(entry.FullName, entryName, StringComparison.OrdinalIgnoreCase))
                    {
                        using (var stream = entry.Open())
                        using (var reader = new System.IO.StreamReader(stream, System.Text.Encoding.UTF8))
                        {
                            return reader.ReadToEnd();
                        }
                    }
                }
            }
            return null;
        }

        // ---------------- 备忘录 ----------------

        private void LoadMemosIntoPanel()
        {
            if (string.IsNullOrEmpty(configPath))
            {
                memoPanel.SetGroups(new List<MemoGroup>());
                return;
            }
            try
            {
                Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                memoPanel.SetGroups(ConfigStore.LoadMemos(root));
            }
            catch
            {
                memoPanel.SetGroups(new List<MemoGroup>());
            }
        }

        /// <summary>备忘录里点了命令：发给当前标签的会话（没会话就提示）。</summary>
        private void OnRunMemoCommand(string cmd, bool execute)
        {
            ISession session = FindSession(tabs.SelectedTab);
            if (session == null || !session.IsConnected)
            {
                SetStatus("没有活动会话，先连接一台主机再执行命令");
                return;
            }
            session.SendText(execute ? cmd + "\r" : cmd);
            SetStatus(execute ? "已执行: " + cmd : "已插入: " + cmd);
        }

        /// <summary>备忘录被改过：写回 config.json。</summary>
        private void OnMemoChanged()
        {
            if (string.IsNullOrEmpty(configPath)) return;
            try
            {
                Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                ConfigStore.SaveMemos(root, memoPanel.CurrentGroups);
                ConfigStore.WriteRoot(configPath, root);
                SetStatus("备忘录已保存");
            }
            catch (Exception ex)
            {
                SetStatus("保存备忘录失败: " + ex.Message);
            }
        }

        // ---------------- 主机管理 ----------------

        private List<string> GetGroups()
        {
            var groups = new List<string>();
            foreach (TreeNode node in hostTree.Nodes)
            {
                string g = node.Tag as string;
                if (!string.IsNullOrEmpty(g) && !groups.Contains(g))
                {
                    groups.Add(g);
                }
            }
            return groups;
        }

        private void OnHostMenuAdd(object sender, EventArgs e)
        {
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("没有配置文件，无法新增（请先「选择配置...」）");
                return;
            }
            string group = null;
            if (contextNode != null)
            {
                var ctxHost = contextNode.Tag as HostEntry;
                group = ctxHost != null ? ctxHost.Group : (contextNode.Tag as string);
            }
            var editor = new HostEditor(null, GetGroups(), group);
            if (editor.ShowDialog(this) == DialogResult.OK)
            {
                SaveHost(editor.Result, true);
            }
        }

        private void OnHostMenuEdit(object sender, EventArgs e)
        {
            var entry = SelectedHost;
            if (entry == null)
            {
                SetStatus("请先选择一台主机");
                return;
            }
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("没有配置文件，无法编辑");
                return;
            }
            var editor = new HostEditor(entry, GetGroups(), entry.Group);
            if (editor.ShowDialog(this) == DialogResult.OK)
            {
                SaveHost(editor.Result, false);
            }
        }

        private void OnHostMenuCopyAddress(object sender, EventArgs e)
        {
            var entry = SelectedHost;
            if (entry == null) return;
            string text = (string.IsNullOrEmpty(entry.User) ? "" : entry.User + "@")
                + entry.Host + ":" + entry.Port;
            try
            {
                Clipboard.SetText(text);
                SetStatus("已复制 " + text);
            }
            catch
            {
                SetStatus("复制失败（剪贴板被占用）");
            }
        }

        private void OnHostMenuRenameGroup(object sender, EventArgs e)
        {
            if (contextNode == null || !(contextNode.Tag is string)) return;
            string oldGroup = (string)contextNode.Tag;
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("没有配置文件，无法重命名");
                return;
            }

            var dlg = new InputDialog("重命名分组", "分组名称：", oldGroup);
            if (dlg.ShowDialog(this) != DialogResult.OK) return;
            string newGroup = dlg.Value;
            if (newGroup.Length == 0 || newGroup == oldGroup) return;

            try
            {
                var root = ConfigStore.ReadRoot(configPath);
                var devices = root["devices"] as System.Collections.IList;
                if (devices != null)
                {
                    foreach (object item in devices)
                    {
                        var dev = item as Dictionary<string, object>;
                        if (dev != null && Convert.ToString(dev["group"]) == oldGroup)
                        {
                            dev["group"] = newGroup;
                            break;
                        }
                    }
                }
                ConfigStore.WriteRoot(configPath, root);
                LoadConfig();
                SetStatus("分组已重命名为 " + newGroup);
            }
            catch (Exception ex)
            {
                SetStatus("重命名失败: " + ex.Message);
            }
        }

        // ---------------- 延迟探测 ----------------

        private void OnHostMenuProbeLatency(object sender, EventArgs e)
        {
            var entry = SelectedHost;
            if (entry == null) return;
            ProbeLatency(entry);
        }

        /// <summary>TCP 连接耗时（后台线程，不阻塞界面）。</summary>
        private void ProbeLatency(HostEntry entry)
        {
            SetStatus("正在测试 " + entry.DisplayName + " 的延迟...");

            System.Threading.ThreadPool.QueueUserWorkItem(delegate
            {
                int ms = -1;
                try
                {
                    var watch = System.Diagnostics.Stopwatch.StartNew();
                    using (var socket = new System.Net.Sockets.TcpClient())
                    {
                        var ar = socket.BeginConnect(entry.Host, entry.Port, null, null);
                        if (ar.AsyncWaitHandle.WaitOne(4000))
                        {
                            socket.EndConnect(ar);
                            ms = (int)watch.ElapsedMilliseconds;
                        }
                    }
                }
                catch
                {
                    ms = -1;
                }

                try
                {
                    BeginInvoke((MethodInvoker)delegate
                    {
                        entry.LatencyMs = ms;
                        UpdateHostNodeLabel(entry);
                        SetStatus(ms >= 0
                            ? entry.DisplayName + " 延迟 " + ms + " ms"
                            : entry.DisplayName + " 连接超时");
                    });
                }
                catch
                {
                }
            });
        }

        /// <summary>只更新某个主机节点的显示文本（不重建整棵树，避免丢探测结果）。</summary>
        private void UpdateHostNodeLabel(HostEntry entry)
        {
            foreach (TreeNode groupNode in hostTree.Nodes)
            {
                foreach (TreeNode hostNode in groupNode.Nodes)
                {
                    var h = hostNode.Tag as HostEntry;
                    if (h == null) continue;
                    if (!ReferenceEquals(h, entry) && h.Host != entry.Host) continue;

                    string label = h.DisplayName;
                    if (h.LatencyMs >= 0) label += "   [" + h.LatencyMs + "ms]";
                    hostNode.Text = label;
                    return;
                }
            }
        }

        /// <summary>新建分组：加到 devices 里一个空 hosts 的节点。</summary>
        private void OnHostMenuNewGroup(object sender, EventArgs e)
        {
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("没有配置文件，无法新建分组");
                return;
            }
            var dlg = new InputDialog("新建分组", "分组名称：", "");
            if (dlg.ShowDialog(this) != DialogResult.OK) return;
            if (dlg.Value.Length == 0) return;

            try
            {
                Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                var devices = root["devices"] as System.Collections.IList;
                if (devices == null)
                {
                    devices = new System.Collections.ArrayList();
                    root["devices"] = devices;
                }
                foreach (object item in devices)
                {
                    var dev = item as Dictionary<string, object>;
                    if (dev != null && Convert.ToString(dev["group"]) == dlg.Value)
                    {
                        SetStatus("分组「" + dlg.Value + "」已存在");
                        return;
                    }
                }
                var device = new Dictionary<string, object>();
                device["group"] = dlg.Value;
                device["hosts"] = new System.Collections.ArrayList();
                devices.Add(device);

                ConfigStore.WriteRoot(configPath, root);
                LoadConfig();
                SetStatus("已新建分组 " + dlg.Value);
            }
            catch (Exception ex)
            {
                SetStatus("新建分组失败: " + ex.Message);
            }
        }

        /// <summary>删除分组（组内还有主机时要求确认）。</summary>
        private void OnHostMenuDeleteGroup(object sender, EventArgs e)
        {
            if (contextNode == null || !(contextNode.Tag is string)) return;
            string group = (string)contextNode.Tag;
            if (string.IsNullOrEmpty(configPath)) return;

            int count = contextNode.Nodes.Count;
            string prompt = count > 0
                ? "分组「" + group + "」里还有 " + count + " 台主机，删除后它们也会一并移除。确定？"
                : "确定删除空分组「" + group + "」？";
            if (MessageBox.Show(this, prompt, "删除分组", MessageBoxButtons.YesNo,
                MessageBoxIcon.Warning) != DialogResult.Yes)
            {
                return;
            }

            try
            {
                Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                var devices = root["devices"] as System.Collections.IList;
                if (devices != null)
                {
                    for (int i = devices.Count - 1; i >= 0; i--)
                    {
                        var dev = devices[i] as Dictionary<string, object>;
                        if (dev != null && Convert.ToString(dev["group"]) == group)
                        {
                            devices.RemoveAt(i);
                        }
                    }
                }
                ConfigStore.WriteRoot(configPath, root);
                LoadConfig();
                SetStatus("已删除分组 " + group);
            }
            catch (Exception ex)
            {
                SetStatus("删除分组失败: " + ex.Message);
            }
        }

        private void OnHostMenuDuplicate(object sender, EventArgs e)
        {
            var entry = SelectedHost;
            if (entry == null)
            {
                SetStatus("请先选择一台主机");
                return;
            }
            var copy = new HostEntry();
            copy.Name = entry.Name + " 副本";
            copy.Group = entry.Group;
            copy.Protocol = entry.Protocol;
            copy.Host = entry.Host;
            copy.Port = entry.Port;
            copy.User = entry.User;
            copy.Pwd = entry.Pwd;
            copy.KeyPath = entry.KeyPath;
            SaveHost(copy, true);
        }

        private void OnHostMenuDelete(object sender, EventArgs e)
        {
            var entry = SelectedHost;
            if (entry == null)
            {
                SetStatus("请先选择一台主机");
                return;
            }
            if (string.IsNullOrEmpty(configPath))
            {
                SetStatus("没有配置文件，无法删除");
                return;
            }

            string name = string.IsNullOrEmpty(entry.Name) ? entry.Host : entry.Name;
            var answer = MessageBox.Show(this,
                "确定删除主机「" + name + "」？", "删除确认",
                MessageBoxButtons.YesNo, MessageBoxIcon.Question);
            if (answer != DialogResult.Yes)
            {
                return;
            }

            try
            {
                var root = ConfigStore.ReadRoot(configPath);
                var devices = root["devices"] as System.Collections.IList;
                if (devices != null && entry.DeviceIndex >= 0 && entry.DeviceIndex < devices.Count)
                {
                    var device = devices[entry.DeviceIndex] as Dictionary<string, object>;
                    var hosts = device == null ? null : device["hosts"] as System.Collections.IList;
                    if (hosts != null && entry.HostIndex >= 0 && entry.HostIndex < hosts.Count)
                    {
                        hosts.RemoveAt(entry.HostIndex);
                    }
                }
                ConfigStore.WriteRoot(configPath, root);
                LoadConfig();
                SetStatus("已删除 " + name + "（config.json.bak 是删除前的备份）");
            }
            catch (Exception ex)
            {
                SetStatus("删除失败: " + ex.Message);
            }
        }

        /// <summary>把主机写回 config.json：新建就追加，编辑就更新原节点（未知字段保留）。</summary>
        private void SaveHost(HostEntry entry, bool isNew)
        {
            if (entry == null || string.IsNullOrEmpty(configPath))
            {
                return;
            }

            try
            {
                var root = ConfigStore.ReadRoot(configPath);
                var devices = root["devices"] as System.Collections.IList;
                if (devices == null)
                {
                    devices = new System.Collections.ArrayList();
                    root["devices"] = devices;
                }

                // 找到（或创建）目标分组
                Dictionary<string, object> target = null;
                for (int i = 0; i < devices.Count; i++)
                {
                    var dev = devices[i] as Dictionary<string, object>;
                    if (dev != null && Convert.ToString(dev["group"]) == entry.Group)
                    {
                        target = dev;
                        break;
                    }
                }
                if (target == null)
                {
                    target = new Dictionary<string, object>();
                    target["group"] = entry.Group;
                    target["hosts"] = new System.Collections.ArrayList();
                    devices.Add(target);
                }

                var hosts = target["hosts"] as System.Collections.IList;
                if (hosts == null)
                {
                    hosts = new System.Collections.ArrayList();
                    target["hosts"] = hosts;
                }

                if (isNew || entry.Raw == null)
                {
                    hosts.Add(BuildRaw(entry));
                }
                else
                {
                    ApplyToRaw(entry);
                    // 换了分组：从旧分组移除，加到新分组
                    bool moved = entry.DeviceIndex >= 0 && entry.DeviceIndex < devices.Count
                        && !ReferenceEquals(devices[entry.DeviceIndex], target);
                    if (moved)
                    {
                        var oldDevice = devices[entry.DeviceIndex] as Dictionary<string, object>;
                        var oldHosts = oldDevice == null ? null : oldDevice["hosts"] as System.Collections.IList;
                        if (oldHosts != null)
                        {
                            oldHosts.Remove(entry.Raw);
                        }
                        hosts.Add(entry.Raw);
                    }
                }

                ConfigStore.WriteRoot(configPath, root);
                LoadConfig();
                SetStatus("已保存 " + entry.DisplayName);
            }
            catch (Exception ex)
            {
                SetStatus("保存失败: " + ex.Message);
            }
        }

        private static Dictionary<string, object> BuildRaw(HostEntry e)
        {
            var raw = new Dictionary<string, object>();
            raw["name"] = e.Name;
            raw["protocol"] = e.Protocol;
            raw["host"] = e.Host;
            raw["port"] = e.Port;
            raw["user"] = e.User;
            raw["pwd"] = e.Pwd;
            raw["key_path"] = e.KeyPath;
            raw["serial_data_bits"] = e.SerialDataBits;
            raw["serial_parity"] = e.SerialParity;
            raw["serial_stop_bits"] = e.SerialStopBits;
            raw["serial_flow_control"] = e.SerialFlowControl;
            raw["serial_dtr"] = e.SerialDtr;
            raw["serial_rts"] = e.SerialRts;
            raw["proxy_host"] = e.ProxyHost;
            raw["proxy_port"] = e.ProxyPort;
            raw["proxy_user"] = e.ProxyUser;
            raw["proxy_pwd"] = e.ProxyPwd;
            raw["proxy_key"] = e.ProxyKey;
            SetMemo(raw, e.Memo);
            return raw;
        }

        /// <summary>只改本程序负责的字段，其余（details、passphrase 等）原样保留。</summary>
        private static void ApplyToRaw(HostEntry e)
        {
            if (e.Raw == null)
            {
                return;
            }
            e.Raw["name"] = e.Name;
            e.Raw["protocol"] = e.Protocol;
            e.Raw["host"] = e.Host;
            e.Raw["port"] = e.Port;
            e.Raw["user"] = e.User;
            e.Raw["pwd"] = e.Pwd;
            e.Raw["key_path"] = e.KeyPath;
            e.Raw["serial_data_bits"] = e.SerialDataBits;
            e.Raw["serial_parity"] = e.SerialParity;
            e.Raw["serial_stop_bits"] = e.SerialStopBits;
            e.Raw["serial_flow_control"] = e.SerialFlowControl;
            e.Raw["serial_dtr"] = e.SerialDtr;
            e.Raw["serial_rts"] = e.SerialRts;
            e.Raw["proxy_host"] = e.ProxyHost;
            e.Raw["proxy_port"] = e.ProxyPort;
            e.Raw["proxy_user"] = e.ProxyUser;
            e.Raw["proxy_pwd"] = e.ProxyPwd;
            e.Raw["proxy_key"] = e.ProxyKey;
            SetMemo(e.Raw, e.Memo);
        }

        /// <summary>备注放在 details.memo（和桌面版同一位置），details 不存在就建。</summary>
        private static void SetMemo(Dictionary<string, object> raw, string memo)
        {
            object detailsObj;
            Dictionary<string, object> details;
            if (!raw.TryGetValue("details", out detailsObj)
                || (details = detailsObj as Dictionary<string, object>) == null)
            {
                details = new Dictionary<string, object>();
                raw["details"] = details;
            }
            details["memo"] = memo == null ? "" : memo;
        }

        // ---------------- 连接 ----------------

        private void OnHostNodeDoubleClick(object sender, TreeNodeMouseClickEventArgs e)
        {
            if (e.Node != null && e.Node.Tag is HostEntry)
            {
                ConnectSelected();
            }
        }

        private void OnConnectClick(object sender, EventArgs e)
        {
            ConnectSelected();
        }

        private void ConnectSelected()
        {
            HostEntry entry = SelectedHost;
            if (entry == null)
            {
                SetStatus("请先在左侧选择一台主机");
                return;
            }

            ISession existing = FindRunningSession(entry);
            if (existing != null)
            {
                if (existing.Page != null)
                {
                    tabs.SelectedTab = existing.Page;
                }
                SetStatus(existing.Entry.DisplayName + " 已经在标签里了");
                return;
            }

            SetStatus(string.Format("正在连接 {0}（{1}:{2}）...", entry.DisplayName, entry.Host, entry.Port));

            ISession session;
            if (entry.Protocol == "serial")
            {
                session = new SerialSession(entry, 80, 24);
            }
            else
            {
                var ssh = new SshSession(entry, 80, 24);
                ssh.AutoReconnect = autoReconnect;
                ssh.LogEnabled = sessionLogEnabled;
                session = ssh;
            }
            session.Message += OnStatusMessage;
            string error;
            if (!session.Connect(out error))
            {
                session.Dispose();
                SetStatus("连接失败: " + error);
                return;
            }

            var page = new TabPage(entry.DisplayName);
            page.ImageIndex = 0;
            session.View.Dock = DockStyle.Fill;
            page.Controls.Add(session.View);
            session.Page = page;

            ApplyAppearance(session);
            sessions.Add(session);
            tabs.TabPages.Add(page);

            suppressBodyRefresh = true;
            try
            {
                tabs.SelectedTab = page;
                hostTree.SelectedNode = null;   // 清掉选中，右侧保持终端
            }
            finally
            {
                suppressBodyRefresh = false;
            }
            SetBody(1);
            UpdateChrome();
            SetStatus(string.Format("已连接 {0}（{1}）", entry.DisplayName, entry.Host));
            session.View.Focus();
        }

        /// <summary>拖拽主机：换分组或组内排序，落定后写回 config.json 的顺序。</summary>
        private void OnHostTreeDragDrop(object sender, DragEventArgs e)
        {
            var source = e.Data.GetData(typeof(TreeNode)) as TreeNode;
            if (source == null || !(source.Tag is HostEntry)) return;

            Point pt = hostTree.PointToClient(new Point(e.X, e.Y));
            TreeNode target = hostTree.GetNodeAt(pt);
            if (target == null || target.Parent == null)
            {
                return;   // 只允许落到分组内部
            }

            TreeNode targetGroupNode = target.Parent;
            HostEntry moving = (HostEntry)source.Tag;

            if (target == source) return;

            int insertAt;
            if (target.Tag is HostEntry)
            {
                insertAt = targetGroupNode.Nodes.IndexOf(target);
                Rectangle r = target.Bounds;
                if (pt.Y > r.Y + r.Height / 2) insertAt++;
            }
            else
            {
                insertAt = targetGroupNode.Nodes.Count;
            }

            hostTree.BeginUpdate();
            try
            {
                source.Parent.Nodes.Remove(source);
                if (insertAt > targetGroupNode.Nodes.Count) insertAt = targetGroupNode.Nodes.Count;
                if (insertAt < 0) insertAt = 0;
                targetGroupNode.Nodes.Insert(insertAt, source);

                // 换组时同步 Group 字段
                string newGroup = targetGroupNode.Tag as string;
                if (newGroup != null && moving.Group != newGroup)
                {
                    moving.Group = newGroup;
                }
                targetGroupNode.Expand();
            }
            finally
            {
                hostTree.EndUpdate();
            }

            hostTree.SelectedNode = source;
            SaveHostOrder();
        }

        /// <summary>按当前树的顺序重建 config.json 的 devices/hosts 数组。</summary>
        private void SaveHostOrder()
        {
            if (string.IsNullOrEmpty(configPath)) return;
            try
            {
                Dictionary<string, object> root = ConfigStore.ReadRoot(configPath);
                var oldDevices = root["devices"] as System.Collections.IList;
                var newDevices = new System.Collections.ArrayList();

                foreach (TreeNode groupNode in hostTree.Nodes)
                {
                    string group = groupNode.Tag as string;
                    if (group == null) continue;

                    // 复用原 device，保住 device 上的其它字段
                    Dictionary<string, object> device = null;
                    if (oldDevices != null)
                    {
                        foreach (object item in oldDevices)
                        {
                            var dev = item as Dictionary<string, object>;
                            if (dev != null && Convert.ToString(dev["group"]) == group)
                            {
                                device = dev;
                                break;
                            }
                        }
                    }
                    if (device == null)
                    {
                        device = new Dictionary<string, object>();
                        device["group"] = group;
                    }

                    var hosts = new System.Collections.ArrayList();
                    foreach (TreeNode hostNode in groupNode.Nodes)
                    {
                        var entry = hostNode.Tag as HostEntry;
                        if (entry == null) continue;
                        hosts.Add(entry.Raw != null ? (object)entry.Raw : BuildRaw(entry));
                    }
                    device["hosts"] = hosts;
                    newDevices.Add(device);
                }

                root["devices"] = newDevices;
                ConfigStore.WriteRoot(configPath, root);
                LoadConfig();
                SetStatus("主机顺序已保存");
            }
            catch (Exception ex)
            {
                SetStatus("保存顺序失败: " + ex.Message);
            }
        }

        private ISession FindRunningSession(HostEntry entry)
        {
            foreach (ISession s in sessions)
            {
                if (s.Entry.SameTarget(entry) && s.IsConnected)
                {
                    return s;
                }
            }
            return null;
        }

        private ISession FindSession(TabPage page)
        {
            if (page == null)
            {
                return null;
            }
            foreach (ISession s in sessions)
            {
                if (s.Page == page)
                {
                    return s;
                }
            }
            return null;
        }

        // ---------------- 标签操作 ----------------

        private static ImageList tabIcons;

        /// <summary>标签上的 × 图标（当作关闭按钮用）。</summary>
        private static ImageList CreateTabIcons()
        {
            if (tabIcons != null)
            {
                return tabIcons;
            }
            var list = new ImageList();
            list.ImageSize = new Size(12, 12);
            list.ColorDepth = ColorDepth.Depth32Bit;

            var bmp = new Bitmap(12, 12);
            using (var g = Graphics.FromImage(bmp))
            {
                g.Clear(Color.Transparent);
                using (var pen = new Pen(Color.FromArgb(120, 120, 120), 1.6f))
                {
                    g.DrawLine(pen, 2.5f, 2.5f, 9.5f, 9.5f);
                    g.DrawLine(pen, 9.5f, 2.5f, 2.5f, 9.5f);
                }
            }
            list.Images.Add(bmp);

            tabIcons = list;
            return list;
        }

        /// <summary>标签上 × 图标的点击区域。</summary>
        private static Rectangle IconRect(Rectangle tabRect)
        {
            int size = 14;
            return new Rectangle(
                tabRect.Left + 3,
                tabRect.Top + (tabRect.Height - size) / 2,
                size, size);
        }

        private void OnTabsMouseDown(object sender, MouseEventArgs e)
        {
            // 临时调试：显示点击坐标和第一个标签的位置，方便校准图标区域
            if (tabs.TabCount > 0)
            {
                SetStatus("click=" + e.X + "," + e.Y + "  tab0=" + tabs.GetTabRect(0)
                    + "  icon=" + IconRect(tabs.GetTabRect(0)));
            }

            for (int i = 0; i < tabs.TabCount; i++)
            {
                Rectangle rect = tabs.GetTabRect(i);
                // 图标区域放宽：标签左侧 28px、整高都算
                var hit = new Rectangle(rect.Left, rect.Top, 28, rect.Height);

                if (e.Button == MouseButtons.Left && hit.Contains(e.Location))
                {
                    ClosePage(tabs.TabPages[i]);
                    return;
                }
                if (e.Button == MouseButtons.Middle && rect.Contains(e.Location))
                {
                    ClosePage(tabs.TabPages[i]);
                    return;
                }
            }
        }

        private void OnTabSelected(object sender, EventArgs e)
        {
            ISession session = FindSession(tabs.SelectedTab);
            if (session != null)
            {
                session.View.Focus();
            }
            UpdateChrome();
        }

        private void OnTabsMouseUp(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Right)
            {
                return;
            }
            TabPage page = FindPageAt(e.Location);
            if (page == null)
            {
                return;
            }
            tabs.SelectedTab = page;
            tabMenu.Show(tabs, e.Location);
        }

        private TabPage FindPageAt(Point location)
        {
            for (int i = 0; i < tabs.TabCount; i++)
            {
                if (tabs.GetTabRect(i).Contains(location))
                {
                    return tabs.TabPages[i];
                }
            }
            return null;
        }

        private void OnDisconnectClick(object sender, EventArgs e)
        {
            if (tabs.SelectedTab == null)
            {
                SetStatus("当前没有会话");
                return;
            }
            ClosePage(tabs.SelectedTab);
        }

        private void OnMenuCloseCurrent(object sender, EventArgs e)
        {
            ClosePage(tabs.SelectedTab);
        }

        private void OnMenuCloseOthers(object sender, EventArgs e)
        {
            TabPage keep = tabs.SelectedTab;
            var closing = new List<ISession>();
            foreach (ISession s in sessions)
            {
                if (s.Page != keep)
                {
                    closing.Add(s);
                }
            }
            foreach (ISession s in closing)
            {
                ClosePage(s.Page);
            }
        }

        private void OnMenuCloseAll(object sender, EventArgs e)
        {
            var closing = new List<ISession>(sessions);
            foreach (ISession s in closing)
            {
                ClosePage(s.Page);
            }
        }

        private void ClosePage(TabPage page)
        {
            ISession session = FindSession(page);
            if (session == null)
            {
                return;
            }

            sessions.Remove(session);
            tabs.TabPages.Remove(page);
            session.Dispose();

            UpdateChrome();

            if (tabs.TabCount == 0)
            {
                SetStatus("会话已全部关闭");
            }
        }

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            // Ctrl+W 关掉当前标签（焦点在终端里时会被终端吃掉，属可接受局限）
            if (keyData == (Keys.Control | Keys.W))
            {
                if (tabs.SelectedTab != null)
                {
                    ClosePage(tabs.SelectedTab);
                    return true;
                }
            }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        // ---------------- 界面状态 ----------------

        private void UpdateChrome()
        {
            RefreshBody();

            bool hasTabs = sessions.Count > 0;
            sessionCountLabel.Text = hasTabs ? sessions.Count + " 个会话" : "无会话";
            disconnectButton.Enabled = tabs.SelectedTab != null && tabs.Visible;
        }

        /// <summary>右侧显示什么：欢迎页 / 会话标签。</summary>
        private void RefreshBody()
        {
            if (suppressBodyRefresh)
            {
                return;
            }
            if (sessions.Count > 0 && tabs.SelectedTab != null)
            {
                SetBody(1);
                return;
            }
            SetBody(0);
        }

        private void SetBody(int mode)
        {
            welcomePanel.Visible = mode == 0;
            tabs.Visible = mode == 1;
        }

        private void SetStatus(string text)
        {
            statusLabel.Text = text;
        }

        private void OnFormClosing(object sender, FormClosingEventArgs e)
        {
            var all = new List<ISession>(sessions);
            sessions.Clear();
            foreach (ISession s in all)
            {
                s.Dispose();
            }
        }
    }
}
