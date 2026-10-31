// 备忘录命令编辑：名称 + 命令内容 + 所属分组（对齐桌面版的表单）。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class MemoEditDialog : Form
    {
        private readonly TextBox nameBox = new TextBox();
        private readonly TextBox cmdBox = new TextBox();
        private readonly ComboBox groupBox = new ComboBox();
        private readonly Button okButton = new Button();
        private readonly Button cancelButton = new Button();

        public string NameValue;
        public string CmdValue;
        public string GroupValue;

        public MemoEditDialog(string title, string name, string cmd, List<string> groups, string group)
        {
            Text = title;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            StartPosition = FormStartPosition.CenterParent;
            MaximizeBox = false;
            MinimizeBox = false;
            ClientSize = new Size(460, 300);
            Font = new Font("Segoe UI", 9f);
            try
            {
                if (Environment.OSVersion.Version.Major < 6) Font = SystemFonts.DefaultFont;
            }
            catch { }

            var nameLabel = new Label();
            nameLabel.Text = "名称";
            nameLabel.AutoSize = true;
            nameLabel.Location = new Point(20, 20);
            Controls.Add(nameLabel);

            nameBox.Location = new Point(90, 17);
            nameBox.Size = new Size(350, 24);
            nameBox.Text = name ?? "";
            Controls.Add(nameBox);

            var groupLabel = new Label();
            groupLabel.Text = "分组";
            groupLabel.AutoSize = true;
            groupLabel.Location = new Point(20, 56);
            Controls.Add(groupLabel);

            groupBox.DropDownStyle = ComboBoxStyle.DropDown;
            groupBox.Location = new Point(90, 53);
            groupBox.Size = new Size(200, 24);
            if (groups != null)
            {
                foreach (string g in groups) groupBox.Items.Add(g);
            }
            groupBox.Text = group ?? "";
            Controls.Add(groupBox);

            var cmdLabel = new Label();
            cmdLabel.Text = "命令";
            cmdLabel.AutoSize = true;
            cmdLabel.Location = new Point(20, 92);
            Controls.Add(cmdLabel);

            cmdBox.Multiline = true;
            cmdBox.ScrollBars = ScrollBars.Vertical;
            cmdBox.Location = new Point(90, 90);
            cmdBox.Size = new Size(350, 140);
            cmdBox.Text = cmd ?? "";
            cmdBox.Font = new Font("Consolas", 9.5f);
            Controls.Add(cmdBox);

            var hint = new Label();
            hint.Text = "名称留空时自动使用命令内容";
            hint.AutoSize = true;
            hint.ForeColor = Color.Gray;
            hint.Location = new Point(90, 236);
            Controls.Add(hint);

            okButton.Text = "确定";
            okButton.Size = new Size(80, 28);
            okButton.Location = new Point(272, ClientSize.Height - 42);
            okButton.Click += OnOk;
            Controls.Add(okButton);

            cancelButton.Text = "取消";
            cancelButton.Size = new Size(80, 28);
            cancelButton.Location = new Point(360, ClientSize.Height - 42);
            cancelButton.DialogResult = DialogResult.Cancel;
            Controls.Add(cancelButton);

            AcceptButton = okButton;
            CancelButton = cancelButton;

            if (name == null && cmd == null)
            {
                cmdBox.Focus();
            }
        }

        private void OnOk(object sender, EventArgs e)
        {
            CmdValue = cmdBox.Text.Trim();
            if (CmdValue.Length == 0)
            {
                MessageBox.Show(this, "命令内容不能为空", "提示", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                cmdBox.Focus();
                return;
            }
            NameValue = nameBox.Text.Trim();
            if (NameValue.Length == 0) NameValue = CmdValue;
            GroupValue = groupBox.Text.Trim();

            DialogResult = DialogResult.OK;
            Close();
        }
    }
}
