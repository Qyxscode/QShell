// 通用设置（与"外观设置"分开）：连接相关的开关。
// 值存 config.json 的 settings：auto_reconnect / session_log_enabled

using System;
using System.Drawing;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class AppSettingsDialog : Form
    {
        private readonly CheckBox reconnectBox = new CheckBox();
        private readonly CheckBox logBox = new CheckBox();
        private readonly Button okButton = new Button();
        private readonly Button cancelButton = new Button();

        public bool AutoReconnectValue;
        public bool SessionLogValue;

        /// <summary>用户在对话框里点了导出/导入（关闭后由宿主执行）。</summary>
        public bool RequestExport;
        public bool RequestImport;

        public AppSettingsDialog(bool autoReconnect, bool sessionLog)
        {
            Text = "设置";
            FormBorderStyle = FormBorderStyle.FixedDialog;
            StartPosition = FormStartPosition.CenterParent;
            MaximizeBox = false;
            MinimizeBox = false;
            ClientSize = new Size(400, 330);
            Font = new Font("Segoe UI", 9f);
            try
            {
                if (Environment.OSVersion.Version.Major < 6) Font = SystemFonts.DefaultFont;
            }
            catch { }

            var group = new GroupBox();
            group.Text = "连接";
            group.Location = new Point(16, 14);
            group.Size = new Size(368, 110);
            Controls.Add(group);

            reconnectBox.Text = "断线后自动重连（每 3 秒重试一次）";
            reconnectBox.AutoSize = true;
            reconnectBox.Location = new Point(16, 28);
            reconnectBox.Checked = autoReconnect;
            group.Controls.Add(reconnectBox);

            logBox.Text = "记录会话日志到 %APPDATA%\\QShell\\logs";
            logBox.AutoSize = true;
            logBox.Location = new Point(16, 60);
            logBox.Checked = sessionLog;
            group.Controls.Add(logBox);

            var backupGroup = new GroupBox();
            backupGroup.Text = "备份";
            backupGroup.Location = new Point(16, 132);
            backupGroup.Size = new Size(368, 92);
            Controls.Add(backupGroup);

            var exportButton = new Button();
            exportButton.Text = "导出配置...";
            exportButton.Size = new Size(120, 28);
            exportButton.Location = new Point(16, 28);
            exportButton.Click += delegate
            {
                RequestExport = true;
                DialogResult = DialogResult.OK;
                Close();
            };
            backupGroup.Controls.Add(exportButton);

            var importButton = new Button();
            importButton.Text = "导入配置...";
            importButton.Size = new Size(120, 28);
            importButton.Location = new Point(148, 28);
            importButton.Click += delegate
            {
                RequestImport = true;
                DialogResult = DialogResult.OK;
                Close();
            };
            backupGroup.Controls.Add(importButton);

            var backupHint = new Label();
            backupHint.Text = "zip（内含 qshell_config.json）或 json，与桌面版通用";
            backupHint.AutoSize = true;
            backupHint.ForeColor = Color.Gray;
            backupHint.Location = new Point(14, 62);
            backupGroup.Controls.Add(backupHint);

            okButton.Text = "确定";
            okButton.Size = new Size(80, 28);
            okButton.Location = new Point(212, ClientSize.Height - 40);
            okButton.Click += OnOk;
            Controls.Add(okButton);

            cancelButton.Text = "取消";
            cancelButton.Size = new Size(80, 28);
            cancelButton.Location = new Point(300, ClientSize.Height - 40);
            cancelButton.DialogResult = DialogResult.Cancel;
            Controls.Add(cancelButton);

            AcceptButton = okButton;
            CancelButton = cancelButton;
        }

        private void OnOk(object sender, EventArgs e)
        {
            AutoReconnectValue = reconnectBox.Checked;
            SessionLogValue = logBox.Checked;
            DialogResult = DialogResult.OK;
            Close();
        }
    }
}
