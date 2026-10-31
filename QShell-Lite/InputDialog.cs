// 简易输入对话框（重命名分组等用）。

using System;
using System.Drawing;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class InputDialog : Form
    {
        private readonly TextBox box = new TextBox();

        public string Value
        {
            get { return box.Text.Trim(); }
        }

        public InputDialog(string title, string prompt, string initial)
        {
            Text = title;
            FormBorderStyle = FormBorderStyle.FixedDialog;
            StartPosition = FormStartPosition.CenterParent;
            MaximizeBox = false;
            MinimizeBox = false;
            ClientSize = new Size(360, 130);
            Font = new Font("Segoe UI", 9f);
            try
            {
                if (Environment.OSVersion.Version.Major < 6) Font = SystemFonts.DefaultFont;
            }
            catch { }

            var label = new Label();
            label.Text = prompt;
            label.AutoSize = true;
            label.Location = new Point(16, 16);
            Controls.Add(label);

            box.Text = initial ?? "";
            box.Location = new Point(16, 40);
            box.Size = new Size(328, 24);
            Controls.Add(box);

            var ok = new Button();
            ok.Text = "确定";
            ok.Size = new Size(80, 28);
            ok.Location = new Point(180, 84);
            ok.DialogResult = DialogResult.OK;
            Controls.Add(ok);

            var cancel = new Button();
            cancel.Text = "取消";
            cancel.Size = new Size(80, 28);
            cancel.Location = new Point(268, 84);
            cancel.DialogResult = DialogResult.Cancel;
            Controls.Add(cancel);

            AcceptButton = ok;
            CancelButton = cancel;
            box.SelectAll();
        }
    }
}
