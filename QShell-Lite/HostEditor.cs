// 主机编辑对话框：新增 / 编辑一条主机。
// 协议选 serial 时，"主机"变成串口选择、"端口"变成波特率，并显示串口参数。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class HostEditor : Form
    {
        private readonly TextBox nameBox = new TextBox();
        private readonly TextBox hostBox = new TextBox();
        private readonly ComboBox portBox = new ComboBox();
        private readonly TextBox userBox = new TextBox();
        private readonly TextBox pwdBox = new TextBox();
        private readonly TextBox keyBox = new TextBox();
        private readonly TextBox memoBox = new TextBox();
        private readonly ComboBox protocolBox = new ComboBox();
        private readonly ComboBox groupBox = new ComboBox();
        private readonly Button okButton = new Button();
        private readonly Button cancelButton = new Button();

        private readonly Label hostLabel = new Label();
        private readonly Label portLabel = new Label();
        private readonly GroupBox serialGroup = new GroupBox();
        private readonly ComboBox dataBitsBox = new ComboBox();
        private readonly ComboBox parityBox = new ComboBox();
        private readonly ComboBox stopBitsBox = new ComboBox();
        private readonly ComboBox flowBox = new ComboBox();
        private readonly CheckBox dtrBox = new CheckBox();
        private readonly CheckBox rtsBox = new CheckBox();

        private readonly GroupBox proxyGroup = new GroupBox();
        private readonly TextBox proxyHostBox = new TextBox();
        private readonly TextBox proxyPortBox = new TextBox();
        private readonly TextBox proxyUserBox = new TextBox();
        private readonly TextBox proxyPwdBox = new TextBox();
        private readonly TextBox proxyKeyBox = new TextBox();

        private readonly HostEntry original;

        public HostEditor(HostEntry entry, IList<string> groups, string defaultGroup)
        {
            original = entry;
            Text = entry == null ? "新增主机" : "编辑主机";
            FormBorderStyle = FormBorderStyle.FixedDialog;
            StartPosition = FormStartPosition.CenterParent;
            MaximizeBox = false;
            MinimizeBox = false;
            ClientSize = new Size(440, 620);
            Font = new Font("Segoe UI", 9f);
            try
            {
                if (Environment.OSVersion.Version.Major < 6) Font = SystemFonts.DefaultFont;
            }
            catch { }

            int y = 12;
            AddRow("名称", nameBox, ref y);

            hostLabel.Text = "主机";
            hostLabel.AutoSize = true;
            hostLabel.Location = new Point(24, y + 4);
            Controls.Add(hostLabel);
            hostBox.Location = new Point(110, y);
            hostBox.Size = new Size(300, 24);
            Controls.Add(hostBox);
            y += 32;

            portLabel.Text = "端口";
            portLabel.AutoSize = true;
            portLabel.Location = new Point(24, y + 4);
            Controls.Add(portLabel);
            portBox.Location = new Point(110, y);
            portBox.Size = new Size(140, 24);
            portBox.DropDownStyle = ComboBoxStyle.DropDown;
            Controls.Add(portBox);
            y += 32;

            AddRow("用户", userBox, ref y);

            pwdBox.UseSystemPasswordChar = true;
            AddRow("密码", pwdBox, ref y);
            AddRow("密钥(.ppk)", keyBox, ref y);

            var memoLabel = new Label();
            memoLabel.Text = "备注";
            memoLabel.AutoSize = true;
            memoLabel.Location = new Point(24, y + 4);
            Controls.Add(memoLabel);
            memoBox.Multiline = true;
            memoBox.ScrollBars = ScrollBars.Vertical;
            memoBox.Location = new Point(110, y);
            memoBox.Size = new Size(300, 46);
            Controls.Add(memoBox);
            y += 54;

            var protoLabel = new Label();
            protoLabel.Text = "协议";
            protoLabel.AutoSize = true;
            protoLabel.Location = new Point(24, y + 4);
            Controls.Add(protoLabel);
            protocolBox.DropDownStyle = ComboBoxStyle.DropDownList;
            protocolBox.Location = new Point(110, y);
            protocolBox.Size = new Size(120, 24);
            protocolBox.Items.AddRange(new object[] { "ssh", "serial", "telnet", "rlogin", "raw" });
            protocolBox.SelectedIndexChanged += delegate { SyncProtocolUi(); };
            Controls.Add(protocolBox);

            var groupLabel = new Label();
            groupLabel.Text = "分组";
            groupLabel.AutoSize = true;
            groupLabel.Location = new Point(250, y + 4);
            Controls.Add(groupLabel);
            groupBox.DropDownStyle = ComboBoxStyle.DropDown;
            groupBox.Location = new Point(296, y);
            groupBox.Size = new Size(114, 24);
            if (groups != null)
            {
                foreach (string g in groups) groupBox.Items.Add(g);
            }
            Controls.Add(groupBox);
            y += 34;

            // ---------- 串口参数 ----------
            serialGroup.Text = "串口参数";
            serialGroup.Location = new Point(16, y);
            serialGroup.Size = new Size(408, 92);
            Controls.Add(serialGroup);

            AddSerialLabel("数据位", 18, 24);
            dataBitsBox.Location = new Point(70, 21);
            dataBitsBox.Size = new Size(60, 24);
            dataBitsBox.DropDownStyle = ComboBoxStyle.DropDownList;
            dataBitsBox.Items.AddRange(new object[] { "5", "6", "7", "8" });
            serialGroup.Controls.Add(dataBitsBox);

            AddSerialLabel("校验", 150, 24);
            parityBox.Location = new Point(200, 21);
            parityBox.Size = new Size(70, 24);
            parityBox.DropDownStyle = ComboBoxStyle.DropDownList;
            parityBox.Items.AddRange(new object[] { "none", "odd", "even" });
            serialGroup.Controls.Add(parityBox);

            AddSerialLabel("停止位", 290, 24);
            stopBitsBox.Location = new Point(340, 21);
            stopBitsBox.Size = new Size(50, 24);
            stopBitsBox.DropDownStyle = ComboBoxStyle.DropDownList;
            stopBitsBox.Items.AddRange(new object[] { "1", "2" });
            serialGroup.Controls.Add(stopBitsBox);

            AddSerialLabel("流控", 18, 58);
            flowBox.Location = new Point(70, 55);
            flowBox.Size = new Size(110, 24);
            flowBox.DropDownStyle = ComboBoxStyle.DropDownList;
            flowBox.Items.AddRange(new object[] { "none", "software", "hardware" });
            serialGroup.Controls.Add(flowBox);

            dtrBox.Text = "DTR";
            dtrBox.AutoSize = true;
            dtrBox.Location = new Point(200, 57);
            serialGroup.Controls.Add(dtrBox);

            rtsBox.Text = "RTS";
            rtsBox.AutoSize = true;
            rtsBox.Location = new Point(290, 57);
            serialGroup.Controls.Add(rtsBox);

            // ---------- 跳板机 ----------
            proxyGroup.Text = "跳板机（留空则直连）";
            proxyGroup.Location = new Point(16, y + 100);
            proxyGroup.Size = new Size(408, 150);
            Controls.Add(proxyGroup);

            AddProxyLabel("地址", 18, 24);
            proxyHostBox.Location = new Point(70, 21);
            proxyHostBox.Size = new Size(160, 24);
            proxyGroup.Controls.Add(proxyHostBox);

            AddProxyLabel("端口", 244, 24);
            proxyPortBox.Location = new Point(296, 21);
            proxyPortBox.Size = new Size(80, 24);
            proxyGroup.Controls.Add(proxyPortBox);

            AddProxyLabel("用户", 18, 56);
            proxyUserBox.Location = new Point(70, 53);
            proxyUserBox.Size = new Size(160, 24);
            proxyGroup.Controls.Add(proxyUserBox);

            AddProxyLabel("密码", 244, 56);
            proxyPwdBox.UseSystemPasswordChar = true;
            proxyPwdBox.Location = new Point(296, 53);
            proxyPwdBox.Size = new Size(80, 24);
            proxyGroup.Controls.Add(proxyPwdBox);

            AddProxyLabel("密钥", 18, 88);
            proxyKeyBox.Location = new Point(70, 85);
            proxyKeyBox.Size = new Size(306, 24);
            proxyGroup.Controls.Add(proxyKeyBox);

            okButton.Text = "确定";
            okButton.Size = new Size(80, 28);
            okButton.Location = new Point(256, ClientSize.Height - 42);
            okButton.Click += OnOk;
            Controls.Add(okButton);

            cancelButton.Text = "取消";
            cancelButton.Size = new Size(80, 28);
            cancelButton.Location = new Point(344, ClientSize.Height - 42);
            cancelButton.DialogResult = DialogResult.Cancel;
            Controls.Add(cancelButton);

            AcceptButton = okButton;
            CancelButton = cancelButton;

            // ---------- 初始值 ----------
            if (entry != null)
            {
                nameBox.Text = entry.Name;
                hostBox.Text = entry.Host;
                portBox.Text = entry.Port.ToString();
                userBox.Text = entry.User;
                pwdBox.Text = entry.Pwd;
                keyBox.Text = entry.KeyPath;
                memoBox.Text = entry.Memo;
                protocolBox.SelectedItem = string.IsNullOrEmpty(entry.Protocol) ? "ssh" : entry.Protocol;
                if (protocolBox.SelectedIndex < 0) protocolBox.SelectedIndex = 0;
                groupBox.Text = entry.Group;
            }
            else
            {
                portBox.Text = "22";
                protocolBox.SelectedIndex = 0;
                if (!string.IsNullOrEmpty(defaultGroup))
                {
                    groupBox.Text = defaultGroup;
                }
                else
                {
                    groupBox.Text = "";   // 空白处新建：不替用户选，留空即"未分组"
                }
            }

            dataBitsBox.SelectedItem = (entry != null ? entry.SerialDataBits : 8).ToString();
            if (dataBitsBox.SelectedIndex < 0) dataBitsBox.SelectedItem = "8";
            parityBox.SelectedItem = entry != null && entry.SerialParity.Length > 0 ? entry.SerialParity : "none";
            if (parityBox.SelectedIndex < 0) parityBox.SelectedIndex = 0;
            stopBitsBox.SelectedItem = (entry != null ? entry.SerialStopBits : 1).ToString();
            if (stopBitsBox.SelectedIndex < 0) stopBitsBox.SelectedItem = "1";
            flowBox.SelectedItem = entry != null && entry.SerialFlowControl.Length > 0 ? entry.SerialFlowControl : "none";
            if (flowBox.SelectedIndex < 0) flowBox.SelectedIndex = 0;
            if (entry != null)
            {
                proxyHostBox.Text = entry.ProxyHost;
                proxyPortBox.Text = entry.ProxyPort.ToString();
                proxyUserBox.Text = entry.ProxyUser;
                proxyPwdBox.Text = entry.ProxyPwd;
                proxyKeyBox.Text = entry.ProxyKey;
            }
            else
            {
                proxyPortBox.Text = "22";
            }

            dtrBox.Checked = entry == null || entry.SerialDtr;
            rtsBox.Checked = entry == null || entry.SerialRts;
        }

        /// <summary>编辑结果（取消时为 null）。</summary>
        public HostEntry Result;

        private void SyncProtocolUi()
        {
            bool serial = (protocolBox.SelectedItem as string) == "serial";
            serialGroup.Visible = serial;

            if (serial)
            {
                hostLabel.Text = "串口";
                portLabel.Text = "波特率";
                portBox.Items.Clear();
                try
                {
                    foreach (string name in SerialSession.ListPorts()) portBox.Items.Add(name);
                }
                catch { }
                if (original == null || original.Protocol != "serial")
                {
                    hostBox.Text = "";
                    portBox.Text = "9600";
                }
            }
            else
            {
                hostLabel.Text = "主机";
                portLabel.Text = "端口";
                if (original == null || original.Protocol == "serial")
                {
                    portBox.Text = "22";
                }
            }
        }

        private void AddRow(string label, TextBox box, ref int y)
        {
            var lb = new Label();
            lb.Text = label;
            lb.AutoSize = true;
            lb.Location = new Point(24, y + 4);
            Controls.Add(lb);

            box.Location = new Point(110, y);
            box.Size = new Size(300, 24);
            Controls.Add(box);

            y += 32;
        }

        private void AddProxyLabel(string text, int x, int y)
        {
            var lb = new Label();
            lb.Text = text;
            lb.AutoSize = true;
            lb.Location = new Point(x, y + 4);
            proxyGroup.Controls.Add(lb);
        }

        private void AddSerialLabel(string text, int x, int y)
        {
            var lb = new Label();
            lb.Text = text;
            lb.AutoSize = true;
            lb.Location = new Point(x, y + 4);
            serialGroup.Controls.Add(lb);
        }

        private void OnOk(object sender, EventArgs e)
        {
            if (hostBox.Text.Trim().Length == 0)
            {
                MessageBox.Show(this, "主机/串口不能为空", "提示", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                hostBox.Focus();
                return;
            }

            int port;
            if (!int.TryParse(portBox.Text.Trim(), out port) || port <= 0 || port > 1000000)
            {
                MessageBox.Show(this, "端口/波特率必须是数字", "提示", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                portBox.Focus();
                return;
            }

            var result = new HostEntry();
            if (Result != null)
            {
                result.DeviceIndex = Result.DeviceIndex;
                result.HostIndex = Result.HostIndex;
                result.Raw = Result.Raw;
            }

            result.Name = nameBox.Text.Trim();
            result.Host = hostBox.Text.Trim();
            result.Port = port;
            result.User = userBox.Text.Trim();
            result.Pwd = pwdBox.Text;
            result.KeyPath = keyBox.Text.Trim();
            result.Memo = memoBox.Text;
            result.Protocol = (protocolBox.SelectedItem as string) ?? "ssh";
            result.Group = groupBox.Text.Trim();
            if (result.Name.Length == 0) result.Name = result.Host;

            result.ProxyHost = proxyHostBox.Text.Trim();
            int proxyPort;
            if (!int.TryParse(proxyPortBox.Text.Trim(), out proxyPort) || proxyPort <= 0)
            {
                proxyPort = 22;
            }
            result.ProxyPort = proxyPort;
            result.ProxyUser = proxyUserBox.Text.Trim();
            result.ProxyPwd = proxyPwdBox.Text;
            result.ProxyKey = proxyKeyBox.Text.Trim();

            if (result.Protocol == "serial")
            {
                result.SerialDataBits = int.Parse((dataBitsBox.SelectedItem as string) ?? "8");
                result.SerialParity = (parityBox.SelectedItem as string) ?? "none";
                result.SerialStopBits = int.Parse((stopBitsBox.SelectedItem as string) ?? "1");
                result.SerialFlowControl = (flowBox.SelectedItem as string) ?? "none";
                result.SerialDtr = dtrBox.Checked;
                result.SerialRts = rtsBox.Checked;
            }

            Result = result;
            DialogResult = DialogResult.OK;
            Close();
        }
    }
}
