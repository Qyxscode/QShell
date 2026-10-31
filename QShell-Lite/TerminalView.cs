// 终端渲染控件：把 VtTerminal 的单元格画出来（GDI+ 双缓冲）。
// spike 阶段不做脏矩形，只做频率限制；htop/vim 刷新率低，够用。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Text;
using System.Text;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class TerminalView : Control
    {
        private readonly VtTerminal term;
        private Font regFont;
        private Font boldFont;
        private int cellW;
        private int cellH;
        private readonly Timer paintTimer;
        private readonly VScrollBar scrollBar = new VScrollBar();
        private readonly Timer blinkTimer = new Timer();
        private bool cursorOn = true;
        private volatile bool dirty;

        /// <summary>光标颜色（宿主可改）。</summary>
        public Color CursorColor = Color.FromArgb(220, 220, 220, 220);

        public event EventHandler ScrollbackChanged;

        public TerminalView(VtTerminal terminal)
        {
            term = terminal;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint
                     | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            BackColor = Color.FromArgb(0x0A, 0x0B, 0x0E);
            TabStop = true;
            ImeMode = ImeMode.On;   // 让中文输入法能往终端里送字

            ApplyFontCore(null, 10f, false);

            paintTimer = new Timer();
            paintTimer.Interval = 30;
            paintTimer.Tick += OnPaintTick;
            paintTimer.Start();

            scrollBar.Dock = DockStyle.Right;
            scrollBar.Width = SystemInformation.VerticalScrollBarWidth;
            scrollBar.Visible = false;
            scrollBar.Scroll += delegate
            {
                term.ViewOffset = Math.Max(0, MaxScroll - scrollBar.Value);
                Invalidate();
            };
            Controls.Add(scrollBar);

            // 光标闪烁
            blinkTimer.Interval = 500;
            blinkTimer.Tick += delegate
            {
                if (term.CursorBlink && term.CursorVisible && term.ViewOffset == 0)
                {
                    cursorOn = !cursorOn;
                    Invalidate();
                }
                else if (!cursorOn)
                {
                    cursorOn = true;
                    Invalidate();
                }
            };
            blinkTimer.Start();

            term.Damaged += MarkDirty;
        }

        /// <summary>实际文本区宽度（扣掉滚动条）。</summary>
        public int ViewWidth
        {
            get { return Width - (scrollBar.Visible ? scrollBar.Width : 0); }
        }

        private int VisibleRows
        {
            get { return Math.Max(1, Height / cellH); }
        }

        private int MaxScroll
        {
            get { return Math.Max(0, scrollBar.Maximum - scrollBar.LargeChange + 1); }
        }

        private void UpdateScrollBar()
        {
            int total = term.ScrollbackCount;
            bool needed = total > 0;
            if (scrollBar.Visible != needed)
            {
                scrollBar.Visible = needed;
            }
            if (!needed)
            {
                return;
            }
            scrollBar.Maximum = Math.Max(0, total - 1);
            scrollBar.LargeChange = Math.Max(1, VisibleRows);

            // ViewOffset 是"向上翻多少"，滚动条 Value 是"向下走多少"，方向相反
            int maxScroll = MaxScroll;
            int value = Math.Max(0, maxScroll - term.ViewOffset);
            if (value > maxScroll) value = maxScroll;
            if (scrollBar.Value != value)
            {
                scrollBar.Value = value;
            }
        }

        /// <summary>换字体并重算单元格尺寸（外观设置用）。</summary>
        public void ApplyAppearance(string family, float size, bool bold,
            int fg, int bg, Color cursor, string cursorStyle, bool cursorBlink)
        {
            ApplyFontCore(family, size, bold);
            term.CursorStyle = cursorStyle == "underline" ? 1 : (cursorStyle == "bar" ? 2 : 0);
            term.CursorBlink = cursorBlink;
            term.DefaultFg = fg;
            term.DefaultBg = bg;
            BackColor = Color.FromArgb(bg);
            CursorColor = cursor;
            Invalidate();
            if (SizeChangedHandlers != null) SizeChangedHandlers();
        }

        private void ApplyFontCore(string family, float size, bool allBold)
        {
            // allBold=true 时正文也用粗体（设置里的"粗体"开关）
            Font newReg = PickFont(family, size, allBold);
            Font newBold = PickFont(family, size, true);
            if (regFont != null) regFont.Dispose();
            if (boldFont != null) boldFont.Dispose();
            regFont = newReg;
            boldFont = newBold;

            cellW = TextRenderer.MeasureText("MMMMMMMMMMMMMMMMMMMM", regFont).Width / 20;
            cellH = regFont.Height;
            if (cellW < 4) cellW = 8;
            if (cellH < 8) cellH = 16;
        }

        private static Font PickFont(string family, float size, bool bold)
        {
            string[] names = { "Consolas", "Lucida Console", "Courier New" };
            foreach (string name in names)
            {
                try
                {
                    var font = new Font(name, size, bold ? FontStyle.Bold : FontStyle.Regular);
                    if (font.Name == name)
                    {
                        return font;
                    }
                    font.Dispose();
                }
                catch
                {
                }
            }
            return new Font(FontFamily.GenericMonospace, size, bold ? FontStyle.Bold : FontStyle.Regular);
        }

        public int CellWidth { get { return cellW; } }
        public int CellHeight { get { return cellH; } }

        private void MarkDirty()
        {
            dirty = true;
        }

        private void OnPaintTick(object sender, EventArgs e)
        {
            if (dirty)
            {
                dirty = false;
                UpdateScrollBar();
                Invalidate();
            }
        }

        public void NotifyScrollbackChanged()
        {
            if (ScrollbackChanged != null) ScrollbackChanged(this, EventArgs.Empty);
        }

        protected override void OnResize(EventArgs e)
        {
            base.OnResize(e);
            if (SizeChangedHandlers != null)
            {
                SizeChangedHandlers();
            }
        }

        /// <summary>尺寸变化时回调（用于通知 SSH 端窗口大小）。</summary>
        public Action SizeChangedHandlers;

        private Line GetLine(int displayRow)
        {
            int sb = term.ScrollbackCount;
            int start = sb - term.ViewOffset;
            int idx = start + displayRow;
            if (idx < 0) return null;
            if (idx < sb) return term.ScrollbackLine(idx);
            int screenRow = idx - sb;
            if (screenRow < 0 || screenRow >= term.Rows) return null;
            return term.Screen[screenRow];
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;

            int bgDefault = term.DefaultBg;
            int fgDefault = term.DefaultFg;
            using (var bgBrush = new SolidBrush(Color.FromArgb(bgDefault)))
            {
                g.FillRectangle(bgBrush, ClientRectangle);
            }

            // 用向下取整：向上取整会让最后一列画到面板外面被裁掉
            int rows = Math.Max(1, Height / cellH);
            int cols = Math.Max(1, ViewWidth / cellW);

            using (var brush = new SolidBrush(Color.Black))
            using (var pen = new Pen(Color.Gray))
            {
                int absBase = ScrollbackTop;
                for (int r = 0; r < rows; r++)
                {
                    Line line = GetLine(r);
                    if (line == null) continue;
                    int y = r * cellH;
                    int absRow = absBase + r;

                    for (int c = 0; c < cols && c < line.Cells.Length; c++)
                    {
                        Cell cell = line.Cells[c];
                        if (cell.Width == 0) continue;

                        bool reverse = (cell.Flags & VtTerminal.FlagReverse) != 0;
                        // -1 才是"用默认色"；ARGB 颜色值为负数，不能用 >= 0 判断
                        int bg = cell.Bg != -1 ? cell.Bg : bgDefault;
                        int fg = cell.Fg != -1 ? cell.Fg : fgDefault;
                        bool isLink = cell.LinkId != 0;
                        if (isLink && cell.Fg == -1) fg = unchecked((int)0xFF6CB6FF);   // 链接默认蓝色
                        if (reverse) { int t = bg; bg = fg; fg = t; }

                        int w = cell.Width == 2 ? cellW * 2 : cellW;
                        if (bg != bgDefault)
                        {
                            brush.Color = Color.FromArgb(bg);
                            g.FillRectangle(brush, c * cellW, y, w, cellH);
                        }

                        if (InSelection(absRow, c))
                        {
                            brush.Color = Color.FromArgb(0x26, 0x4F, 0x78);
                            g.FillRectangle(brush, c * cellW, y, w, cellH);
                        }
                        else if (IsSearchHit(absRow, c))
                        {
                            brush.Color = Color.FromArgb(0x6B, 0x5A, 0x00);
                            g.FillRectangle(brush, c * cellW, y, w, cellH);
                        }

                        if (cell.Ch == 0) continue;

                        string text = Utf32ToString(cell.Ch);
                        if (cell.Comb != 0) text += Utf32ToString(cell.Comb);
                        bool bold = (cell.Flags & VtTerminal.FlagBold) != 0;
                        brush.Color = Color.FromArgb(fg);
                        g.DrawString(text, bold ? boldFont : regFont, brush, c * cellW, y);

                        if (isLink || (cell.Flags & VtTerminal.FlagUnderline) != 0)
                        {
                            pen.Color = Color.FromArgb(fg);
                            g.DrawLine(pen, c * cellW, y + cellH - 1, c * cellW + w, y + cellH - 1);
                        }
                        if ((cell.Flags & VtTerminal.FlagStrike) != 0)
                        {
                            pen.Color = Color.FromArgb(fg);
                            g.DrawLine(pen, c * cellW, y + cellH / 2, c * cellW + w, y + cellH / 2);
                        }
                    }
                }

                if (term.ViewOffset == 0 && term.CursorVisible && term.CursorRow < rows
                    && (!term.CursorBlink || cursorOn))
                {
                    int cx = term.CursorCol * cellW;
                    int cy = term.CursorRow * cellH;
                    using (var cursorBrush = new SolidBrush(CursorColor))
                    {
                        if (term.CursorStyle == 1)
                        {
                            // 下划线（vim 插入模式常见）
                            g.FillRectangle(cursorBrush, cx, cy + cellH - 2, cellW, 2);
                        }
                        else if (term.CursorStyle == 2)
                        {
                            // 竖线
                            g.FillRectangle(cursorBrush, cx, cy, 2, cellH);
                        }
                        else
                        {
                            g.FillRectangle(cursorBrush, cx, cy, cellW, cellH);
                        }
                    }
                }
            }
        }

        private static string Utf32ToString(uint cp)
        {
            try
            {
                return char.ConvertFromUtf32((int)cp);
            }
            catch
            {
                return "?";
            }
        }

        protected override bool IsInputKey(Keys keyData)
        {
            return true;
        }

        /// <summary>
        /// 方向键/Home/End 直接在消息层处理：WinForms 会把它们当导航键，
        /// 在 Win7 上可能根本到不了 OnKeyDown（实测左键/Home/End 没反应）。
        /// 在 WndProc 里拦下来自己发，跳过整条键盘路由。
        /// </summary>
        protected override void WndProc(ref Message m)
        {
            const int WM_KEYDOWN = 0x0100;
            const int WM_SYSKEYDOWN = 0x0104;

            if (Input != null && (m.Msg == WM_KEYDOWN || m.Msg == WM_SYSKEYDOWN))
            {
                int vk = m.WParam.ToInt32();

                // Alt+字母/数字：发 ESC 前缀（meta 序列），readline 的 Alt+b/f 等靠它
                if (m.Msg == WM_SYSKEYDOWN)
                {
                    string alt = AltSequence(vk);
                    if (alt != null)
                    {
                        Input(Enc(alt));
                        return;
                    }
                }
                else
                {
                    Keys mods = ModifierKeys;

                    // Shift+PageUp/PageDown 翻本地历史，不发给远端
                    if ((mods & Keys.Shift) != 0)
                    {
                        if (vk == 0x21)
                        {
                            term.ScrollView(VisibleRows - 2);
                            UpdateScrollBar();
                            return;
                        }
                        if (vk == 0x22)
                        {
                            term.ScrollView(-(VisibleRows - 2));
                            UpdateScrollBar();
                            return;
                        }
                    }

                    string seq = NavSequence(vk, (mods & Keys.Control) != 0);
                    if (seq != null)
                    {
                        Input(Enc(seq));
                        return;
                    }
                }
            }
            base.WndProc(ref m);
        }

        /// <summary>Alt+字母/数字 → ESC 前缀（readline 的 meta 绑定）。</summary>
        private static string AltSequence(int vk)
        {
            if (vk >= 0x41 && vk <= 0x5A)
            {
                return "\x1b" + char.ToLowerInvariant((char)vk).ToString();
            }
            if (vk >= 0x30 && vk <= 0x39)
            {
                return "\x1b" + ((char)vk).ToString();
            }
            if (vk == 0x08)
            {
                return "\x1b\x7f";     // Alt+Backspace = 删前一个词
            }
            return null;
        }

        private string NavSequence(int vk, bool ctrl)
        {
            // Ctrl+方向：readline / vim 用 xterm 的 "1;5" 修饰形式按词移动
            if (ctrl)
            {
                switch (vk)
                {
                    case 0x25: return "\x1b[1;5D";   // Ctrl+Left
                    case 0x27: return "\x1b[1;5C";   // Ctrl+Right
                    case 0x24: return "\x1b[1;5H";   // Ctrl+Home
                    case 0x23: return "\x1b[1;5F";   // Ctrl+End
                }
                return null;
            }

            bool app = term.ApplicationCursorKeys;
            switch (vk)
            {
                case 0x25: return app ? "\x1bOD" : "\x1b[D";   // Left
                case 0x26: return app ? "\x1bOA" : "\x1b[A";   // Up
                case 0x27: return app ? "\x1bOC" : "\x1b[C";   // Right
                case 0x28: return app ? "\x1bOB" : "\x1b[B";   // Down
                case 0x24: return app ? "\x1bOH" : "\x1b[H";   // Home
                case 0x23: return app ? "\x1bOF" : "\x1b[F";   // End
            }
            return null;
        }

        /// <summary>
        /// 方向键/Home/End/Tab 在 WinForms 里默认被当成"导航键"，
        /// 可能到不了 KeyDown。显式声明"不当作命令键"，让它走 KeyDown。
        /// </summary>
        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            switch (keyData & Keys.KeyCode)
            {
                case Keys.Left:
                case Keys.Right:
                case Keys.Up:
                case Keys.Down:
                case Keys.Home:
                case Keys.End:
                case Keys.Tab:
                    return false;
            }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        // ---------------- 选择 / 复制 ----------------

        private int selAnchorRow = -1;
        private int selAnchorCol;
        private int selActiveRow = -1;
        private int selActiveCol;
        private bool draggingSelection;

        private int ScrollbackTop
        {
            get { return term.ScrollbackCount - term.ViewOffset; }
        }

        private int AbsRowAt(int y)
        {
            int row = Math.Max(0, Math.Min(term.Rows - 1, y / cellH));
            return ScrollbackTop + row;
        }

        private int ColAt(int x)
        {
            return Math.Max(0, Math.Min(term.Cols - 1, x / cellW));
        }

        private Line LineAtAbs(int absRow)
        {
            int sb = term.ScrollbackCount;
            if (absRow < 0) return null;
            if (absRow < sb) return term.ScrollbackLine(absRow);
            int screenRow = absRow - sb;
            if (screenRow < 0 || screenRow >= term.Rows) return null;
            return term.Screen[screenRow];
        }

        public bool HasSelection
        {
            get { return selAnchorRow >= 0 && selActiveRow >= 0; }
        }

        public void ClearSelection()
        {
            selAnchorRow = -1;
            selActiveRow = -1;
            Invalidate();
        }

        public void SelectAll()
        {
            selAnchorRow = 0;
            selAnchorCol = 0;
            selActiveRow = Math.Max(0, term.Rows - 1);
            selActiveCol = term.Cols - 1;
            Invalidate();
        }

        private bool InSelection(int absRow, int col)
        {
            if (!HasSelection) return false;

            int r1 = selAnchorRow, c1 = selAnchorCol;
            int r2 = selActiveRow, c2 = selActiveCol;
            if (r1 > r2 || (r1 == r2 && c1 > c2))
            {
                int t = r1; r1 = r2; r2 = t;
                t = c1; c1 = c2; c2 = t;
            }

            if (absRow < r1 || absRow > r2) return false;
            if (absRow == r1 && col < c1) return false;
            if (absRow == r2 && col > c2) return false;
            return true;
        }

        public string GetSelectedText()
        {
            if (!HasSelection) return "";

            int r1 = selAnchorRow, c1 = selAnchorCol;
            int r2 = selActiveRow, c2 = selActiveCol;
            if (r1 > r2 || (r1 == r2 && c1 > c2))
            {
                int t = r1; r1 = r2; r2 = t;
                t = c1; c1 = c2; c2 = t;
            }

            var sb = new StringBuilder();
            for (int row = r1; row <= r2; row++)
            {
                Line line = LineAtAbs(row);
                if (line == null) continue;

                int from = row == r1 ? c1 : 0;
                int to = row == r2 ? c2 : line.Cells.Length - 1;
                var lineText = new StringBuilder();
                for (int c = from; c <= to && c < line.Cells.Length; c++)
                {
                    Cell cell = line.Cells[c];
                    if (cell.Width == 0) continue;
                    if (cell.Ch < 0x20)
                    {
                        lineText.Append(' ');
                    }
                    else
                    {
                        lineText.Append(char.ConvertFromUtf32((int)cell.Ch));
                    }
                }

                sb.Append(lineText.ToString().TrimEnd());
                if (row != r2)
                {
                    // 自动换行的续行不插换行，恢复成原来的逻辑行
                    Line next = LineAtAbs(row + 1);
                    if (next == null || !next.Wrapped)
                    {
                        sb.Append("\r\n");
                    }
                }
            }
            return sb.ToString();
        }

        public void CopySelection()
        {
            string text = GetSelectedText();
            if (text.Length == 0) return;
            try
            {
                Clipboard.SetText(text);
            }
            catch
            {
            }
        }

        public void PasteClipboard()
        {
            if (Input == null) return;
            string text = null;
            try
            {
                if (Clipboard.ContainsText())
                {
                    text = Clipboard.GetText();
                }
            }
            catch
            {
            }
            if (string.IsNullOrEmpty(text)) return;

            // bracketed paste: 让 vim 之类的程序知道这是粘贴而不是键入
            if (term.BracketedPaste)
            {
                Input(Encoding.UTF8.GetBytes("\x1b[200~" + text + "\x1b[201~"));
            }
            else
            {
                Input(Encoding.UTF8.GetBytes(text));
            }
        }

        protected override void OnMouseDown(MouseEventArgs e)
        {
            base.OnMouseDown(e);
            Focus();

            // Ctrl+点击：打开 OSC 8 超链接
            if ((ModifierKeys & Keys.Control) != 0)
            {
                string uri = LinkAt(e.X, e.Y);
                if (!string.IsNullOrEmpty(uri))
                {
                    try { System.Diagnostics.Process.Start(uri); } catch { }
                    return;
                }
            }

            // 远端程序要鼠标（vim/tmux/htop）时优先发过去；按住 Shift 强制本地选择
            if (MouseToRemote)
            {
                remoteButton = e.Button;
                remoteDragging = true;
                SendMouse(e.Button, true, false, e.X, e.Y);
                Capture = true;
                return;
            }

            if (e.Button != MouseButtons.Left) return;

            // 单击 / 双击 / 三击：三击选整行，双击选词
            if ((DateTime.UtcNow - lastClickTime).TotalMilliseconds < 500)
            {
                clickCount++;
            }
            else
            {
                clickCount = 1;
            }
            lastClickTime = DateTime.UtcNow;

            if (clickCount == 2)
            {
                SelectWordAt(e.X, e.Y);
                return;
            }
            if (clickCount >= 3)
            {
                SelectLineAt(e.Y);
                clickCount = 0;
                return;
            }

            selAnchorRow = AbsRowAt(e.Y);
            selAnchorCol = ColAt(e.X);
            selActiveRow = selAnchorRow;
            selActiveCol = selAnchorCol;
            draggingSelection = true;
            Capture = true;
            Invalidate();
        }

        // ---------------- 双击选词 / 三击选行 ----------------

        private int clickCount;
        private DateTime lastClickTime = DateTime.MinValue;

        private static bool IsWordChar(Cell cell)
        {
            if (cell.Ch == 0 || cell.Width == 0) return false;
            uint c = cell.Ch;
            if (c >= '0' && c <= '9') return true;
            if (c >= 'a' && c <= 'z') return true;
            if (c >= 'A' && c <= 'Z') return true;
            if (c == '_' || c == '-' || c == '.' || c == '/' || c == '~' || c == '@') return true;
            if (c >= 0x4E00 && c <= 0x9FFF) return true;   // 中文
            if (c >= 0x3040 && c <= 0x30FF) return true;   // 日文假名
            return false;
        }

        private void SelectWordAt(int x, int y)
        {
            int abs = AbsRowAt(y);
            Line line = LineAtAbs(abs);
            if (line == null) return;

            int col = Math.Max(0, Math.Min(line.Cells.Length - 1, x / cellW));
            if (line.Cells[col].Width == 0 && col > 0) col--;

            int start = col;
            int end = col;
            while (start > 0 && IsWordChar(line.Cells[start - 1])) start--;
            while (end + 1 < line.Cells.Length && IsWordChar(line.Cells[end + 1])) end++;

            selAnchorRow = abs;
            selAnchorCol = start;
            selActiveRow = abs;
            selActiveCol = end;
            Invalidate();
            CopySelection();
        }

        private void SelectLineAt(int y)
        {
            int abs = AbsRowAt(y);
            Line line = LineAtAbs(abs);
            if (line == null) return;

            selAnchorRow = abs;
            selAnchorCol = 0;
            selActiveRow = abs;
            selActiveCol = line.Cells.Length - 1;
            Invalidate();
            CopySelection();
        }

        protected override void OnMouseMove(MouseEventArgs e)
        {
            base.OnMouseMove(e);

            if (remoteDragging && term.MouseMode >= 2)
            {
                SendMouse(remoteButton, true, true, e.X, e.Y);
                return;
            }

            if (draggingSelection)
            {
                selActiveRow = AbsRowAt(e.Y);
                selActiveCol = ColAt(e.X);
                Invalidate();
                return;
            }

            // 悬停在链接上时变成手型
            Cursor = LinkAt(e.X, e.Y) != null ? Cursors.Hand : Cursors.IBeam;
        }

        protected override void OnMouseUp(MouseEventArgs e)
        {
            base.OnMouseUp(e);

            if (remoteDragging)
            {
                remoteDragging = false;
                Capture = false;
                SendMouse(remoteButton, false, false, e.X, e.Y);
                return;
            }

            if (!draggingSelection) return;
            draggingSelection = false;
            Capture = false;
            CopySelection();      // 选中即复制
        }

        protected override void OnMouseWheel(MouseEventArgs e)
        {
            base.OnMouseWheel(e);

            if (MouseToRemote)
            {
                SendWheel(e.Delta > 0, e.X, e.Y);
                return;
            }
            term.ScrollView(e.Delta > 0 ? 3 : -3);
        }

        // ---------------- 鼠标上报 / 超链接 ----------------

        private MouseButtons remoteButton;
        private bool remoteDragging;

        private bool MouseToRemote
        {
            get { return term.MouseMode > 0 && (ModifierKeys & Keys.Shift) == 0; }
        }

        private string LinkAt(int x, int y)
        {
            int row = Math.Max(0, Math.Min(term.Rows - 1, y / cellH));
            int abs = ScrollbackTop + row;
            Line line = LineAtAbs(abs);
            if (line == null) return null;
            int col = Math.Max(0, Math.Min(line.Cells.Length - 1, x / cellW));
            // 点在宽字符中间时回退到它的首格
            if (line.Cells[col].Width == 0 && col > 0) col--;
            ushort id = line.Cells[col].LinkId;
            return id == 0 ? null : term.GetLink(id);
        }

        private void SendMouse(MouseButtons button, bool press, bool motion, int x, int y)
        {
            if (Input == null) return;

            int code;
            if (button == MouseButtons.Left) code = 0;
            else if (button == MouseButtons.Middle) code = 1;
            else if (button == MouseButtons.Right) code = 2;
            else code = 3;

            if (motion) code += 32;
            if ((ModifierKeys & Keys.Shift) != 0) code += 4;
            if ((ModifierKeys & Keys.Alt) != 0) code += 8;
            if ((ModifierKeys & Keys.Control) != 0) code += 16;

            int col = Math.Max(0, Math.Min(term.Cols - 1, x / cellW));
            int row = Math.Max(0, Math.Min(term.Rows - 1, y / cellH));

            if (term.MouseSgr)
            {
                string seq = "\x1b[<" + code + ";" + (col + 1) + ";" + (row + 1) + (press ? "M" : "m");
                Input(Encoding.ASCII.GetBytes(seq));
            }
            else
            {
                Input(new byte[]
                {
                    0x1B, (byte)'[', (byte)'M',
                    (byte)(32 + code), (byte)(33 + col), (byte)(33 + row)
                });
            }
        }

        private void SendWheel(bool up, int x, int y)
        {
            if (Input == null) return;
            int col = Math.Max(0, Math.Min(term.Cols - 1, x / cellW));
            int row = Math.Max(0, Math.Min(term.Rows - 1, y / cellH));
            int code = up ? 64 : 65;
            if (term.MouseSgr)
            {
                string seq = "\x1b[<" + code + ";" + (col + 1) + ";" + (row + 1) + "M";
                Input(Encoding.ASCII.GetBytes(seq));
            }
            else
            {
                Input(new byte[]
                {
                    0x1B, (byte)'[', (byte)'M',
                    (byte)(32 + code), (byte)(33 + col), (byte)(33 + row)
                });
            }
        }

        // ---------------- 搜索 ----------------

        public event Action SearchRequested;

        private readonly Dictionary<int, List<int[]>> hitsByRow = new Dictionary<int, List<int[]>>();
        private readonly List<int[]> hitList = new List<int[]>();
        private int hitIndex = -1;

        public int SearchHitCount
        {
            get { return hitList.Count; }
        }

        public int SearchHitIndex
        {
            get { return hitIndex; }
        }

        public int Search(string text)
        {
            hitsByRow.Clear();
            hitList.Clear();
            hitIndex = -1;

            string needle = text == null ? "" : text;
            if (needle.Length == 0)
            {
                Invalidate();
                return 0;
            }

            int total = term.ScrollbackCount + term.Rows;
            for (int abs = 0; abs < total; abs++)
            {
                string lineText;
                int[] colMap;
                LineTextWithMap(abs, out lineText, out colMap);
                if (lineText.Length < needle.Length) continue;

                int at = 0;
                while (at <= lineText.Length - needle.Length)
                {
                    int found = lineText.IndexOf(needle, at, StringComparison.OrdinalIgnoreCase);
                    if (found < 0) break;
                    var hit = new int[] { abs, colMap[found], colMap[found + needle.Length - 1] };
                    hitList.Add(hit);
                    List<int[]> rowHits;
                    if (!hitsByRow.TryGetValue(abs, out rowHits))
                    {
                        rowHits = new List<int[]>();
                        hitsByRow[abs] = rowHits;
                    }
                    rowHits.Add(hit);
                    at = found + 1;
                }
            }

            if (hitList.Count > 0)
            {
                hitIndex = 0;
                ScrollToHit(0);
            }
            Invalidate();
            return hitList.Count;
        }

        public void NextHit(bool forward)
        {
            if (hitList.Count == 0) return;
            hitIndex += forward ? 1 : -1;
            if (hitIndex < 0) hitIndex = hitList.Count - 1;
            if (hitIndex >= hitList.Count) hitIndex = 0;
            ScrollToHit(hitIndex);
            Invalidate();
        }

        public void ClearSearch()
        {
            hitsByRow.Clear();
            hitList.Clear();
            hitIndex = -1;
            Invalidate();
        }

        private void ScrollToHit(int index)
        {
            if (index < 0 || index >= hitList.Count) return;
            int absRow = hitList[index][0];
            int sb = term.ScrollbackCount;
            // 让匹配行出现在可视区顶部附近
            term.ViewOffset = absRow < sb ? (sb - absRow) : 0;
        }

        private void LineTextWithMap(int absRow, out string text, out int[] colMap)
        {
            text = "";
            colMap = new int[0];
            Line line = LineAtAbs(absRow);
            if (line == null) return;

            var sb = new StringBuilder();
            var cols = new List<int>();
            for (int c = 0; c < line.Cells.Length; c++)
            {
                Cell cell = line.Cells[c];
                if (cell.Width == 0) continue;
                cols.Add(c);
                if (cell.Ch == 0) { sb.Append(' '); continue; }
                if (cell.Ch > 0xFFFF) { sb.Append('\uFFFD'); continue; }
                sb.Append((char)cell.Ch);
            }
            text = sb.ToString();
            colMap = cols.ToArray();
        }

        private bool IsSearchHit(int absRow, int col)
        {
            List<int[]> rowHits;
            if (!hitsByRow.TryGetValue(absRow, out rowHits)) return false;
            for (int i = 0; i < rowHits.Count; i++)
            {
                if (col >= rowHits[i][1] && col <= rowHits[i][2]) return true;
            }
            return false;
        }

        /// <summary>终端产生的输入字节（由宿主发给 SSH）。</summary>
        public Action<byte[]> Input;

        private static byte[] Enc(string s)
        {
            return Encoding.UTF8.GetBytes(s);
        }

        protected override void OnKeyPress(KeyPressEventArgs e)
        {
            if (Input != null && e.KeyChar >= 0x20 && e.KeyChar != 0x7F)
            {
                Input(Enc(e.KeyChar.ToString()));
                e.Handled = true;
            }
            base.OnKeyPress(e);
        }

        protected override void OnKeyDown(KeyEventArgs e)
        {
            // Ctrl+F：打开搜索栏
            if (e.Control && !e.Shift && e.KeyCode == Keys.F)
            {
                if (SearchRequested != null) SearchRequested();
                e.Handled = true;
                e.SuppressKeyPress = true;
                return;
            }

            // Ctrl+Shift+C/V/A：终端里 Ctrl+C 要留给远端，所以用 Shift 组合
            if (e.Control && e.Shift)
            {
                if (e.KeyCode == Keys.C)
                {
                    CopySelection();
                    e.Handled = true;
                    e.SuppressKeyPress = true;
                    return;
                }
                if (e.KeyCode == Keys.V)
                {
                    PasteClipboard();
                    e.Handled = true;
                    e.SuppressKeyPress = true;
                    return;
                }
                if (e.KeyCode == Keys.A)
                {
                    SelectAll();
                    e.Handled = true;
                    e.SuppressKeyPress = true;
                    return;
                }
            }

            if (Input != null)
            {
                byte[] seq = MapKey(e);
                if (seq != null)
                {
                    Input(seq);
                    e.Handled = true;
                    e.SuppressKeyPress = true;
                }
            }
            base.OnKeyDown(e);
        }

        private byte[] MapKey(KeyEventArgs e)
        {
            switch (e.KeyCode)
            {
                case Keys.Insert: return Enc("\x1b[2~");
                case Keys.Delete: return Enc("\x1b[3~");
                case Keys.PageUp: return Enc("\x1b[5~");
                case Keys.PageDown: return Enc("\x1b[6~");
                case Keys.Enter: return Enc("\r");
                case Keys.Back: return Enc("\x7f");
                case Keys.Escape: return Enc("\x1b");
                case Keys.Tab: return Enc(e.Shift ? "\x1b[Z" : "\t");
                case Keys.F1: return Enc("\x1bOP");
                case Keys.F2: return Enc("\x1bOQ");
                case Keys.F3: return Enc("\x1bOR");
                case Keys.F4: return Enc("\x1bOS");
                case Keys.F5: return Enc("\x1b[15~");
                case Keys.F6: return Enc("\x1b[17~");
                case Keys.F7: return Enc("\x1b[18~");
                case Keys.F8: return Enc("\x1b[19~");
                case Keys.F9: return Enc("\x1b[20~");
                case Keys.F10: return Enc("\x1b[21~");
                case Keys.F11: return Enc("\x1b[23~");
                case Keys.F12: return Enc("\x1b[24~");
            }

            if (e.Control && e.KeyCode >= Keys.A && e.KeyCode <= Keys.Z)
            {
                return new byte[] { (byte)(e.KeyCode - Keys.A + 1) };
            }

            return null;
        }
    }
}
