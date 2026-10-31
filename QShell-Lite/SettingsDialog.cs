// 外观设置：字体 / 字号 / 粗体 / 光标样式 / 光标闪烁 / 字色。
// 值存 config.json 的 settings：font_family / font_size / is_bold /
// cursor_style / cursor_blink / terminal_fg_color

using System;
using System.Drawing;
using System.Drawing.Text;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class SettingsDialog : Form
    {
        private readonly ComboBox fontBox = new ComboBox();
        private readonly NumericUpDown sizeBox = new NumericUpDown();
        private readonly CheckBox boldBox = new CheckBox();
        private readonly ComboBox cursorBox = new ComboBox();
        private readonly CheckBox blinkBox = new CheckBox();
        private readonly Button colorButton = new Button();
        private readonly Button okButton = new Button();
        private readonly Button cancelButton = new Button();

        private Color fgColor;

        public string FontFamilyValue;
        public float FontSizeValue;
        public bool BoldValue;
        public string CursorStyleValue;
        public bool CursorBlinkValue;
        public int ForeColorValue;

        public SettingsDialog(string fontFamily, float fontSize, bool bold,
            string cursorStyle, bool cursorBlink, int fg)
        {
            Text = "外观设置";
            FormBorderStyle = FormBorderStyle.FixedDialog;
            StartPosition = FormStartPosition.CenterParent;
            MaximizeBox = false;
            MinimizeBox = false;
            ClientSize = new Size(420, 300);
            Font = new Font("Segoe UI", 9f);
            try
            {
                if (Environment.OSVersion.Version.Major < 6) Font = SystemFonts.DefaultFont;
            }
            catch { }

            int y = 16;
            AddLabel("字体", y);
            fontBox.DropDownStyle = ComboBoxStyle.DropDownList;
            fontBox.Location = new Point(110, y);
            fontBox.Size = new Size(280, 24);
            FillMonospaceFonts();
            if (!string.IsNullOrEmpty(fontFamily)) fontBox.SelectedItem = fontFamily;
            if (fontBox.SelectedIndex < 0 && fontBox.Items.Count > 0) fontBox.SelectedIndex = 0;
            Controls.Add(fontBox);
            y += 34;

            AddLabel("字号", y);
            sizeBox.Location = new Point(110, y);
            sizeBox.Size = new Size(80, 24);
            sizeBox.Minimum = 6;
            sizeBox.Maximum = 40;
            sizeBox.DecimalPlaces = 0;
            sizeBox.Value = (decimal)Math.Max(6, Math.Min(40, fontSize));
            Controls.Add(sizeBox);
            y += 34;

            boldBox.Text = "粗体";
            boldBox.Location = new Point(110, y);
            boldBox.AutoSize = true;
            boldBox.Checked = bold;
            Controls.Add(boldBox);
            y += 30;

            AddLabel("光标样式", y);
            cursorBox.DropDownStyle = ComboBoxStyle.DropDownList;
            cursorBox.Location = new Point(110, y);
            cursorBox.Size = new Size(120, 24);
            cursorBox.Items.AddRange(new object[] { "block", "underline", "bar" });
            cursorBox.SelectedItem = string.IsNullOrEmpty(cursorStyle) ? "block" : cursorStyle;
            if (cursorBox.SelectedIndex < 0) cursorBox.SelectedIndex = 0;
            Controls.Add(cursorBox);
            y += 34;

            blinkBox.Text = "光标闪烁";
            blinkBox.Location = new Point(110, y);
            blinkBox.AutoSize = true;
            blinkBox.Checked = cursorBlink;
            Controls.Add(blinkBox);
            y += 30;

            AddLabel("字色", y);
            fgColor = Color.FromArgb(fg);
            colorButton.Text = "选择颜色...";
            colorButton.Location = new Point(110, y);
            colorButton.Size = new Size(120, 26);
            colorButton.BackColor = fgColor;
            colorButton.ForeColor = fgColor.GetBrightness() > 0.5f ? Color.Black : Color.White;
            colorButton.Click += OnPickColor;
            Controls.Add(colorButton);
            y += 40;

            okButton.Text = "确定";
            okButton.Size = new Size(80, 28);
            okButton.Location = new Point(232, ClientSize.Height - 42);
            okButton.Click += OnOk;
            Controls.Add(okButton);

            cancelButton.Text = "取消";
            cancelButton.Size = new Size(80, 28);
            cancelButton.Location = new Point(320, ClientSize.Height - 42);
            cancelButton.DialogResult = DialogResult.Cancel;
            Controls.Add(cancelButton);

            AcceptButton = okButton;
            CancelButton = cancelButton;
        }

        private void AddLabel(string text, int y)
        {
            var lb = new Label();
            lb.Text = text;
            lb.AutoSize = true;
            lb.Location = new Point(24, y + 4);
            Controls.Add(lb);
        }

        /// <summary>只列等宽字体（终端必须等宽）。</summary>
        private void FillMonospaceFonts()
        {
            try
            {
                using (var fonts = new InstalledFontCollection())
                {
                    foreach (FontFamily family in fonts.Families)
                    {
                        try
                        {
                            if (!family.IsStyleAvailable(FontStyle.Regular)) continue;
                            using (var f = new Font(family, 10f))
                            {
                                int w1 = TextRenderer.MeasureText("iiiiiiiiii", f).Width;
                                int w2 = TextRenderer.MeasureText("MMMMMMMMMM", f).Width;
                                if (Math.Abs(w1 - w2) <= 2)
                                {
                                    fontBox.Items.Add(family.Name);
                                }
                            }
                        }
                        catch
                        {
                        }
                    }
                }
            }
            catch
            {
            }
        }

        private void OnPickColor(object sender, EventArgs e)
        {
            using (var dlg = new ColorDialog())
            {
                dlg.Color = fgColor;
                dlg.FullOpen = true;
                if (dlg.ShowDialog(this) == DialogResult.OK)
                {
                    fgColor = dlg.Color;
                    colorButton.BackColor = fgColor;
                    colorButton.ForeColor = fgColor.GetBrightness() > 0.5f ? Color.Black : Color.White;
                }
            }
        }

        private void OnOk(object sender, EventArgs e)
        {
            FontFamilyValue = fontBox.SelectedItem as string;
            FontSizeValue = (float)sizeBox.Value;
            BoldValue = boldBox.Checked;
            CursorStyleValue = (cursorBox.SelectedItem as string) ?? "block";
            CursorBlinkValue = blinkBox.Checked;
            ForeColorValue = fgColor.ToArgb();
            DialogResult = DialogResult.OK;
            Close();
        }
    }
}
