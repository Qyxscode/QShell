// VT 终端核心（spike 版）：字节流 -> 屏幕单元格模型。
// 参考 libvterm 的分层（parser/state/screen），为 .NET 4.0 + C# 5 精简重写。
// 覆盖 htop / vim / tmux / less 所需: 光标、SGR(含 256/真彩)、滚动区、备用屏、
// 插入删除行列、自动换行、UTF-8、CJK 双宽、scrollback、标题。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.Text;

namespace QShellLite
{
    internal struct Cell
    {
        public uint Ch;      // 主码位, 0 = 空白
        public uint Comb;    // 附加组合字符(只存一个, 够用)
        public byte Width;   // 1 = 单宽, 2 = 双宽(CJK), 0 = 宽字符占位格
        public byte Flags;
        public int Fg;       // ARGB, -1 = 默认前景
        public int Bg;       // ARGB, -1 = 默认背景
        public ushort LinkId; // OSC 8 超链接编号, 0 = 无
    }

    internal sealed class Line
    {
        /// <summary>
        /// 每行按固定宽存储。变窄时只是"看不见"，内容不会被删掉 ——
        /// 这样远端（bash/readline）在窗口变化后重绘时才有完整内容可对齐。
        /// </summary>
        public const int MaxCols = 400;

        public Cell[] Cells;
        public bool Wrapped;   // true = 接下来那一行是本行的逻辑续行

        public Line(int cols)
        {
            Cells = new Cell[MaxCols];
            // 必须初始化成"默认色"(-1)，否则 Bg 会是 0（透明黑），
            // 渲染时被当成"有背景色"而画出一层不该有的东西。
            for (int i = 0; i < MaxCols; i++)
            {
                Cells[i].Width = 1;
                Cells[i].Fg = -1;
                Cells[i].Bg = -1;
            }
        }
    }

    internal sealed class VtTerminal
    {
        public const byte FlagBold = 1;
        public const byte FlagUnderline = 2;
        public const byte FlagItalic = 4;
        public const byte FlagReverse = 8;
        public const byte FlagStrike = 16;
        public const byte FlagConceal = 32;

        public int Rows = 24;
        public int Cols = 80;

        public Line[] Screen;
        private Line[] altScreen;
        private Line[] mainScreen;      // 备用屏激活时保存主屏
        private int altSavedRow;
        private int altSavedCol;
        private bool usingAlt;

        private readonly List<Line> scrollback = new List<Line>();
        public int ScrollbackLimit = 2000;
        public int ViewOffset;          // >0 表示向上查看历史

        public int CursorRow;
        public int CursorCol;
        public bool CursorVisible = true;
        public bool AutoWrap = true;
        public bool InsertMode;
        public bool BracketedPaste;
        public bool ApplicationCursorKeys = true;   // bash/readline 默认就是应用模式
        public int MouseMode;           // 0=none 1=click 2=drag 3=move
        public bool MouseSgr;           // ?1006 扩展坐标格式

        private readonly List<string> hyperlinks = new List<string>();
        private ushort currentLink;

        /// <summary>OSC 8 超链接表（编号从 1 开始，0 表示无链接）。</summary>
        public string GetLink(ushort id)
        {
            if (id == 0 || id > hyperlinks.Count) return null;
            return hyperlinks[id - 1];
        }

        public int DefaultFg = Color.FromArgb(0xCC, 0xCC, 0xCC).ToArgb();
        public int DefaultBg = Color.FromArgb(0x0A, 0x0B, 0x0E).ToArgb();

        private int penFg = -1;
        private int penBg = -1;
        private byte penFlags;

        private int scrollTop;
        private int scrollBot;

        public string Title = "";
        public event Action TitleChanged;
        /// <summary>内容发生变化，UI 需要重绘。</summary>
        public event Action Damaged;
        /// <summary>远端通过 OSC 52 请求设置剪贴板。</summary>
        public event Action<string> ClipboardRequest;

        /// <summary>光标形状：0/1=方块 2=下划线 3=竖线（DECSCUSR 的简化映射）。</summary>
        public int CursorStyle;
        /// <summary>光标是否闪烁（DECSCUSR 的 1/3/5 都是闪烁；默认闪）。</summary>
        public bool CursorBlink = true;

        /// <summary>
        /// 是否在列数变化时重排（reflow）。
        /// 默认关闭：SSH 会话里远端（readline）收到 SIGWINCH 后会重绘当前命令行，
        /// 和本地的重排互相打架，表现为拖完窗口内容残留/错乱。
        /// PuTTY 也不做 reflow（变窄截断）。要做对需要维护逻辑行并与远端状态协调，
        /// 留作后续工作。
        /// </summary>
        public bool ReflowEnabled = false;

        // ---------- 解析器状态 ----------
        private enum PState { Ground, Escape, Csi, Osc, Dcs, Ss3, Ignore }

        private PState state = PState.Ground;
        private readonly int[] csiArgs = new int[32];
        private int csiArgc;
        private int csiAccum;
        private bool csiHasAccum;
        private char csiPrefix;      // '?' '>' '=' '!'
        private int csiIntermediates;
        private byte csiIntermediateByte;
        private readonly StringBuilder oscText = new StringBuilder();
        private uint pendingHigh;    // UTF-8 解码
        private int pendingNeed;

        private int[] tabStops;
        private bool wrapPending;

        public VtTerminal(int rows, int cols)
        {
            Rows = rows < 1 ? 24 : rows;
            Cols = cols < 1 ? 80 : cols;
            Screen = NewLines(Rows, Cols);
            scrollTop = 0;
            scrollBot = Rows - 1;
            BuildTabStops();
            ClearAll();
        }

        private Line[] NewLines(int rows, int cols)
        {
            var lines = new Line[rows];
            for (int i = 0; i < rows; i++)
            {
                lines[i] = new Line(cols);
            }
            return lines;
        }

        public Line VisibleLine(int row)
        {
            return Screen[row];
        }

        public int ScrollbackCount
        {
            get { return scrollback.Count; }
        }

        public Line ScrollbackLine(int index)
        {
            return scrollback[index];
        }

        private void BuildTabStops()
        {
            tabStops = new int[Cols];
            for (int i = 0; i < Cols; i++)
            {
                tabStops[i] = (i % 8 == 0) ? 1 : 0;
            }
        }

        // ================= 公共入口 =================

        public void Feed(byte[] data, int offset, int count)
        {
            if (data == null)
            {
                return;
            }
            int end = offset + count;
            for (int i = offset; i < end; i++)
            {
                Consume(data[i]);
            }
        }

        /// <summary>强制按新宽度重排（分屏/布局切换时用，普通窗口拖动不走这里）。</summary>
        public void ResizeWithReflow(int rows, int cols)
        {
            bool saved = ReflowEnabled;
            ReflowEnabled = true;
            try
            {
                if (rows == Rows && cols == Cols)
                {
                    // 尺寸没变也要重排一次：前面可能已经被普通 resize 截断过了
                    Reflow(cols);
                }
                else
                {
                    Resize(rows, cols);
                }
            }
            finally
            {
                ReflowEnabled = saved;
            }
        }

        public void Resize(int rows, int cols)
        {
            if (rows < 1) rows = 1;
            if (cols < 1) cols = 1;
            if (rows == Rows && cols == Cols)
            {
                return;
            }

            // 列数变化时先重排（把之前折行的内容合并回逻辑行再重新折行）。
            // 变窄也要做 —— 否则内容被硬截断，再拖宽时就只能合并到残缺的内容。
            // 只在主屏且光标在底部时做：全屏程序（vim/htop）光标在中间，
            // 重排反而会破坏画面，它们的备用屏也不受影响。
            bool reflowed = false;
            if (ReflowEnabled && cols != Cols && !usingAlt)
            {
                Reflow(cols);
                reflowed = true;
            }

            if (reflowed)
            {
                // Reflow 已经按新列宽重建了 Screen（行数还是旧的），
                // 不能再走下面的复制流程 —— 那里的边界用的是旧 Cols，会把内容截掉。
                if (rows != Rows)
                {
                    Screen = ResizeLines(Screen, rows, cols);
                }
            }
            else
            {
                var newScreen = new Line[rows];
                for (int r = 0; r < rows; r++)
                {
                    newScreen[r] = new Line(cols);
                    // Wrapped 必须一起复制 —— 丢了它，下次重排就不知道哪些行是续行
                    if (r < Rows)
                    {
                        newScreen[r].Wrapped = Screen[r].Wrapped;
                    }
                    for (int c = 0; c < Line.MaxCols; c++)
                    {
                        // 复制整行（不只当前可见宽度）：变窄时超出的内容照样保留，
                        // 远端重绘时才能对齐
                        newScreen[r].Cells[c] = r < Rows ? Screen[r].Cells[c] : Blank();
                    }
                }
                Screen = newScreen;
            }
            Rows = rows;
            Cols = cols;
            if (CursorRow >= Rows) CursorRow = Rows - 1;
            if (CursorCol >= Cols) CursorCol = Cols - 1;
            scrollTop = 0;
            scrollBot = Rows - 1;
            BuildTabStops();

            // 备用屏激活时 Screen 指向 altScreen，主屏要单独按新尺寸重排
            if (mainScreen != null)
            {
                mainScreen = ResizeLines(mainScreen, rows, cols);
            }
            if (usingAlt)
            {
                altScreen = Screen;
            }
            else if (altScreen != null)
            {
                altScreen = null;
            }
            if (Damaged != null)
            {
                Damaged();
            }
        }

        /// <summary>
        /// 变宽时重排：把带 Wrapped 标记的续行合并回逻辑行，再按新列数重新折行。
        /// 只在主屏、光标在底部时调用（全屏程序的光标在中间，重排会破坏画面）。
        /// </summary>
        private void Reflow(int newCols)
        {
            var merged = new List<List<Cell>>();

            // 屏幕第一行如果是续行，它的前几段都在 scrollback 里 ——
            // 一路回溯到这条逻辑行的起点，取回所有分段一起参与重排。
            // 只取最后一行是不够的：窄窗口下一条逻辑行可能被折成好几段。
            if (Rows > 0 && Screen[0].Wrapped)
            {
                var tail = new List<Line>();
                while (scrollback.Count > 0)
                {
                    Line sb = scrollback[scrollback.Count - 1];
                    tail.Insert(0, sb);
                    scrollback.RemoveAt(scrollback.Count - 1);
                    if (!sb.Wrapped)
                    {
                        break;
                    }
                }
                if (tail.Count > 0)
                {
                    var carry = new List<Cell>();
                    foreach (Line sb in tail)
                    {
                        int sbLen = sb.Cells.Length;
                        while (sbLen > 0 && sb.Cells[sbLen - 1].Ch == 0) sbLen--;
                        for (int i = 0; i < sbLen; i++) carry.Add(sb.Cells[i]);
                    }
                    merged.Add(carry);
                }
            }

            // 1) 合并屏幕内的续行。每行只取到最后一个实际写入的字符 ——
            //    行尾那一大片空单元格如果也拼进去，内容会被撑开、重排后错乱。
            for (int r = 0; r < Rows; r++)
            {
                Line src = Screen[r];
                int len = src.Cells.Length;
                while (len > 0 && src.Cells[len - 1].Ch == 0) len--;

                if (src.Wrapped && merged.Count > 0)
                {
                    List<Cell> prev = merged[merged.Count - 1];
                    for (int i = 0; i < len; i++) prev.Add(src.Cells[i]);
                }
                else
                {
                    var cells = new List<Cell>(len);
                    for (int i = 0; i < len; i++) cells.Add(src.Cells[i]);
                    merged.Add(cells);
                }
            }

            // 去掉合并结果尾部的空行：屏幕底部的空白行如果也参与重排，
            // 会把真正有内容的行挤到 scrollback 里，屏幕上只剩后半截。
            while (merged.Count > 0 && merged[merged.Count - 1].Count == 0)
            {
                merged.RemoveAt(merged.Count - 1);
            }

            // 2) 按新宽度重折
            var refolded = new List<Line>();
            foreach (List<Cell> cells in merged)
            {
                if (cells.Count == 0)
                {
                    refolded.Add(NewBlankLine());
                    continue;
                }
                for (int start = 0; start < cells.Count; start += newCols)
                {
                    int len = Math.Min(newCols, cells.Count - start);
                    var line = new Line(newCols);
                    for (int i = 0; i < len; i++) line.Cells[i] = cells[start + i];
                    // Wrapped 的语义是"本行是上一行的续行"：只有第一段不是续行，
                    // 其余都是。之前写成"本行后面还有内容"，整个语义反了，
                    // 导致重排一次之后所有行的续行标记全错，第二次拖动就散架。
                    line.Wrapped = start > 0;
                    refolded.Add(line);
                }
            }

            // 3) 填回屏幕：多出来的推到 scrollback，不足的补空行
            int extra = Math.Max(0, refolded.Count - Rows);
            for (int i = 0; i < extra; i++) PushScrollback(refolded[i]);
            int used = Math.Min(refolded.Count, Rows);

            var newScreen = new Line[Rows];
            for (int i = 0; i < used; i++) newScreen[i] = refolded[extra + i];
            for (int i = used; i < Rows; i++) newScreen[i] = NewBlankLine();
            Screen = newScreen;

            CursorRow = Rows - 1;
            Line last = Screen[CursorRow];
            int lastCol = last.Cells.Length - 1;
            while (lastCol > 0 && last.Cells[lastCol].Ch == 0) lastCol--;
            CursorCol = Math.Min(Cols - 1, lastCol + 1);
            ViewOffset = 0;
        }

        private static Line[] ResizeLines(Line[] old, int rows, int cols)
        {
            var lines = new Line[rows];
            for (int r = 0; r < rows; r++)
            {
                lines[r] = new Line(cols);
                if (r < old.Length)
                {
                    lines[r].Wrapped = old[r].Wrapped;
                    for (int c = 0; c < Line.MaxCols && c < old[r].Cells.Length; c++)
                    {
                        lines[r].Cells[c] = old[r].Cells[c];
                    }
                }
            }
            return lines;
        }

        public void ScrollView(int delta)
        {
            int max = scrollback.Count;
            ViewOffset += delta;
            if (ViewOffset < 0) ViewOffset = 0;
            if (ViewOffset > max) ViewOffset = max;
            if (Damaged != null)
            {
                Damaged();
            }
        }

        // ================= 解析器 =================

        private void Consume(byte b)
        {
            if (b >= 0x20 && b < 0x7F && state == PState.Ground)
            {
                if (pendingNeed > 0) { pendingNeed = 0; pendingHigh = 0; }
                PutChar(b);
                return;
            }

            if (b >= 0x80 || pendingNeed > 0)
            {
                ConsumeUtf8(b);
                return;
            }

            if (b < 0x20 || b == 0x7F)
            {
                if (b == 0x1B)
                {
                    // ESC: 任何状态下都回到 Escape（OSC/DCS 里则是提前结束）
                    if (state == PState.Osc)
                    {
                        FinishOsc();
                    }
                    state = PState.Escape;
                    csiPrefix = '\0';
                    csiIntermediates = 0;
                    return;
                }

                if (state == PState.Osc)
                {
                    if (b == 7) { FinishOsc(); return; }
                }
                if (state == PState.Ground)
                {
                    Control(b);
                }
                return;
            }

            switch (state)
            {
                case PState.Ground:
                    if (b == 0x1B) { state = PState.Escape; }
                    break;

                case PState.Escape:
                    if (b == '[') { BeginCsi(); }
                    else if (b == ']') { state = PState.Osc; oscText.Length = 0; oscFirst = true; }
                    else if (b == 'P') { state = PState.Dcs; }
                    else if (b == 'O') { state = PState.Ss3; }
                    else if (b == '(' || b == ')' || b == '*' || b == '+')
                    {
                        // 字符集选择, 忽略一个字节
                        state = PState.Ignore;
                    }
                    else
                    {
                        EscapeSingle((char)b);
                        if (state != PState.Ignore)
                        {
                            state = PState.Ground;
                        }
                    }
                    break;

                case PState.Csi:
                    CsiByte((char)b);
                    break;

                case PState.Osc:
                    if (b == 0x1B)
                    {
                        FinishOsc();
                        state = PState.Escape;
                    }
                    else
                    {
                        if (oscText.Length < 4096) oscText.Append((char)b);
                    }
                    break;

                case PState.Dcs:
                    if (b == 0x1B) { state = PState.Escape; }
                    break;

                case PState.Ss3:
                    Ss3Dispatch((char)b);
                    state = PState.Ground;
                    break;

                case PState.Ignore:
                    state = PState.Ground;
                    break;
            }
        }

        private bool oscFirst;

        private void ConsumeUtf8(byte b)
        {
            if (state == PState.Osc)
            {
                if (oscText.Length < 4096) oscText.Append((char)b);
                return;
            }
            if (state != PState.Ground)
            {
                return;
            }

            if (pendingNeed == 0)
            {
                if (b < 0x80) { PutChar(b); return; }
                if ((b & 0xE0) == 0xC0) { pendingHigh = (uint)(b & 0x1F); pendingNeed = 1; }
                else if ((b & 0xF0) == 0xE0) { pendingHigh = (uint)(b & 0x0F); pendingNeed = 2; }
                else if ((b & 0xF8) == 0xF0) { pendingHigh = (uint)(b & 0x07); pendingNeed = 3; }
                return;
            }

            if ((b & 0xC0) != 0x80)
            {
                pendingNeed = 0;
                pendingHigh = 0;
                return;
            }

            pendingHigh = (pendingHigh << 6) | (uint)(b & 0x3F);
            pendingNeed--;
            if (pendingNeed == 0)
            {
                uint cp = pendingHigh;
                pendingHigh = 0;
                PutChar(cp);
            }
        }

        private void Control(byte b)
        {
            switch (b)
            {
                case 7: break;                       // BEL
                case 8: if (CursorCol > 0) CursorCol--; wrapPending = false; break;   // BS
                case 9: Tab(1); break;               // HT
                case 10: LineFeed(); break;          // LF
                case 11: case 12: LineFeed(); break; // VT/FF
                case 13: CursorCol = 0; wrapPending = false; break;                   // CR
            }
            // 光标位置变了就得重绘 —— readline 移动光标用的是 \b（退格），
            // 不在这里通知界面的话，光标会"位置对了但画面没动"。
            if (Damaged != null) Damaged();
        }

        private void BeginCsi()
        {
            state = PState.Csi;
            csiArgc = 0;
            csiAccum = 0;
            csiHasAccum = false;
            csiPrefix = '\0';
            csiIntermediates = 0;
            for (int i = 0; i < csiArgs.Length; i++) csiArgs[i] = -1;
        }

        private void CsiByte(char c)
        {
            if (c >= '0' && c <= '9')
            {
                csiAccum = csiAccum * 10 + (c - '0');
                if (csiAccum > 65535) csiAccum = 65535;
                csiHasAccum = true;
                return;
            }

            if (c == ';' || c == ':')
            {
                PushCsiArg();
                return;
            }

            if (c >= 0x3C && c <= 0x3F)
            {
                csiPrefix = c;
                return;
            }

            if (c >= 0x20 && c <= 0x2F)
            {
                csiIntermediates++;
                csiIntermediateByte = (byte)c;
                return;
            }

            // 终止字节
            PushCsiArg();
            state = PState.Ground;
            CsiDispatch(c);
        }

        private void PushCsiArg()
        {
            if (csiArgc < csiArgs.Length - 1)
            {
                csiArgs[csiArgc++] = csiHasAccum ? csiAccum : -1;
            }
            csiAccum = 0;
            csiHasAccum = false;
        }

        private int Arg(int index, int fallback)
        {
            if (index >= csiArgc) return fallback;
            int v = csiArgs[index];
            return v < 0 ? fallback : v;
        }

        private int Arg(int index)
        {
            return Arg(index, 0);
        }

        private void CsiDispatch(char cmd)
        {
            if (csiPrefix == '?' && (cmd == 'h' || cmd == 'l'))
            {
                for (int i = 0; i < csiArgc; i++)
                {
                    SetMode(csiArgs[i], cmd == 'h');
                }
                return;
            }

            // CSI Ps SP q = DECSCUSR（设置光标样式），vim 靠它区分插入/普通模式
            if (csiIntermediates > 0)
            {
                if (csiIntermediateByte == 0x20 && cmd == 'q')
                {
                    int ps = Arg(0, 0);
                    if (ps >= 0 && ps <= 6)
                    {
                        CursorStyle = (ps == 3 || ps == 4) ? 1 : (ps == 5 || ps == 6 ? 2 : 0);
                        CursorBlink = (ps == 1 || ps == 3 || ps == 5);
                        if (Damaged != null) Damaged();
                    }
                }
                return;
            }

            // 带私有前缀（> = < ?）的 CSI 不是标准控制序列，一律忽略。
            // 典型坑：xterm 的 ESC[>4;2m（modifyOtherKeys）会被误当成 SGR 解析成
            // 4(下划线) + 2(粗体)，导致整个屏幕的文字都带上下划线。
            if (csiPrefix != '\0')
            {
                return;
            }

            switch (cmd)
            {
                case 'A': MoveCursor(-Arg(0, 1), 0); break;
                case 'B': case 'e': MoveCursor(Arg(0, 1), 0); break;
                case 'C': case 'a': MoveCursor(0, Arg(0, 1)); break;
                case 'D': MoveCursor(0, -Arg(0, 1)); break;
                case 'E': CursorRow = Math.Min(Rows - 1, CursorRow + Arg(0, 1)); CursorCol = 0; break;
                case 'F': CursorRow = Math.Max(0, CursorRow - Arg(0, 1)); CursorCol = 0; break;
                case 'G': case '`': CursorCol = Clamp(Arg(0, 1) - 1, 0, Cols - 1); break;
                case 'd': CursorRow = Clamp(Arg(0, 1) - 1, 0, Rows - 1); break;
                case 'H': case 'f': SetCursor(Arg(0, 1) - 1, Arg(1, 1) - 1); break;
                case 'J': EraseDisplay(Arg(0, 0)); break;
                case 'K': EraseLine(Arg(0, 0)); break;
                case 'L': InsertLines(Arg(0, 1)); break;
                case 'M': DeleteLines(Arg(0, 1)); break;
                case 'P': DeleteChars(Arg(0, 1)); break;
                case 'X': EraseChars(Arg(0, 1)); break;
                case '@': InsertChars(Arg(0, 1)); break;
                case 'S': ScrollUp(Arg(0, 1)); break;
                case 'T': ScrollDown(Arg(0, 1)); break;
                case 'r':
                    scrollTop = Clamp(Arg(0, 1) - 1, 0, Rows - 1);
                    scrollBot = Clamp(Arg(1, Rows) - 1, scrollTop, Rows - 1);
                    SetCursor(0, 0);
                    break;
                case 'm': Sgr(); break;
                case 'n': DeviceStatus(Arg(0, 0)); break;
                case 's': SaveCursor(); break;
                case 'u': RestoreCursor(); break;
                case 'b': RepeatLast(Arg(0, 1)); break;
                case 'g': if (Arg(0, 0) == 3) { for (int i = 0; i < Cols; i++) tabStops[i] = 0; } break;
                case 'Z': Tab(-Arg(0, 1)); break;
                case 'h': case 'l': break;
                default: break;
            }
            wrapPending = false;
            if (Damaged != null) Damaged();
        }

        /// <summary>
        /// SS3 序列（ESC O x）：应用光标键模式下 bash 的方向键就是这种格式。
        /// 不处理的话，按左键会变成"忽略 O + 打印一个 D"。
        /// </summary>
        private void Ss3Dispatch(char ch)
        {
            switch (ch)
            {
                case 'A': MoveCursor(-1, 0); break;
                case 'B': MoveCursor(1, 0); break;
                case 'C': MoveCursor(0, 1); break;
                case 'D': MoveCursor(0, -1); break;
                case 'H': CursorCol = 0; break;
                case 'F': CursorCol = Cols - 1; break;
                default: return;   // F1-F4(P/Q/R/S) 等，不影响光标
            }
            wrapPending = false;
            if (Damaged != null) Damaged();
        }

        private void EscapeSingle(char c)
        {            switch (c)
            {
                case '7': SaveCursor(); break;
                case '8': RestoreCursor(); break;
                case 'D': LineFeed(); break;
                case 'E': CursorCol = 0; LineFeed(); break;
                case 'M': ReverseIndex(); break;
                case 'H': if (CursorCol < Cols) tabStops[CursorCol] = 1; break;
                case 'c': HardReset(); break;
                default: break;
            }
            if (Damaged != null) Damaged();
        }

        private void FinishOsc()
        {
            state = PState.Ground;
            string s = oscText.ToString();
            int semi = s.IndexOf(';');
            string cmdText = semi >= 0 ? s.Substring(0, semi) : s;
            string rest = semi >= 0 ? s.Substring(semi + 1) : "";
            int cmd;
            if (!int.TryParse(cmdText, out cmd))
            {
                return;
            }
            if (cmd == 0 || cmd == 1 || cmd == 2)
            {
                Title = rest;
                if (TitleChanged != null) TitleChanged();
            }
            else if (cmd == 8)
            {
                // OSC 8 ; params ; URI   —— params 里可能带 id=xxx，这里只取 URI
                int uriSep = rest.IndexOf(';');
                string uri = uriSep >= 0 ? rest.Substring(uriSep + 1) : rest;
                uri = uri.Trim();
                if (uri.Length == 0)
                {
                    currentLink = 0;   // 空 URI = 结束当前链接
                }
                else
                {
                    int index = hyperlinks.IndexOf(uri);
                    if (index < 0)
                    {
                        hyperlinks.Add(uri);
                        index = hyperlinks.Count - 1;
                    }
                    currentLink = (ushort)(index + 1);
                }
            }
            else if (cmd == 52)
            {
                // OSC 52 ; c ; base64 —— 远端请求写入系统剪贴板（tmux copy-mode 用）
                int sep = rest.IndexOf(';');
                if (sep >= 0)
                {
                    string b64 = rest.Substring(sep + 1).Trim();
                    if (b64.Length > 0 && b64 != "?")
                    {
                        try
                        {
                            byte[] decoded = Convert.FromBase64String(b64);
                            string text = System.Text.Encoding.UTF8.GetString(decoded);
                            if (ClipboardRequest != null) ClipboardRequest(text);
                        }
                        catch { }
                    }
                }
            }
            oscText.Length = 0;
        }

        // ================= 模式 / 状态 =================

        private void SetMode(int mode, bool enable)
        {
            switch (mode)
            {
                case 4: InsertMode = enable; break;
                case 7: AutoWrap = enable; break;
                case 25: CursorVisible = enable; break;
                case 1: ApplicationCursorKeys = enable; break;
                case 2004: BracketedPaste = enable; break;
                case 47: case 1047: case 1049: SwitchAlt(enable); break;
                case 1000: MouseMode = enable ? 1 : 0; break;
                case 1002: MouseMode = enable ? 2 : 0; break;
                case 1003: MouseMode = enable ? 3 : 0; break;
                case 1006: MouseSgr = enable; break;
                default: break;
            }
            if (Damaged != null) Damaged();
        }

        /// <summary>
        /// 切换主屏/备用屏（vim、tmux、less 都靠它）。
        /// 关键：必须真的切换 Screen 数组，光改标志位等于没切 —— 那样 tmux 退出后
        /// 整屏还是它留下的画面（绿底状态栏之类）。
        /// </summary>
        private void SwitchAlt(bool enable)
        {
            if (enable && !usingAlt)
            {
                altSavedRow = CursorRow;
                altSavedCol = CursorCol;
                mainScreen = Screen;
                if (altScreen == null || altScreen.Length != Rows)
                {
                    altScreen = NewLines(Rows, Cols);
                }
                for (int r = 0; r < Rows; r++)
                {
                    for (int c = 0; c < Cols; c++)
                    {
                        altScreen[r].Cells[c] = Blank();
                    }
                }
                Screen = altScreen;
                usingAlt = true;
                CursorRow = 0;
                CursorCol = 0;
            }
            else if (!enable && usingAlt)
            {
                Screen = mainScreen != null ? mainScreen : NewLines(Rows, Cols);
                mainScreen = null;
                usingAlt = false;
                CursorRow = Clamp(altSavedRow, 0, Rows - 1);
                CursorCol = Clamp(altSavedCol, 0, Cols - 1);
            }
        }

        private void HardReset()
        {
            if (usingAlt && mainScreen != null)
            {
                Screen = mainScreen;
            }
            usingAlt = false;
            altScreen = null;
            mainScreen = null;
            penFg = -1;
            penBg = -1;
            penFlags = 0;
            CursorVisible = true;
            AutoWrap = true;
            InsertMode = false;
            scrollTop = 0;
            scrollBot = Rows - 1;
            scrollback.Clear();
            ViewOffset = 0;
            ClearAll();
            SetCursor(0, 0);
        }

        private void ClearAll()
        {
            for (int r = 0; r < Rows; r++)
                for (int c = 0; c < Cols; c++)
                    Screen[r].Cells[c] = Blank();
        }

        private Cell Blank()
        {
            Cell cell = new Cell();
            cell.Width = 1;
            cell.Fg = -1;
            cell.Bg = -1;
            return cell;
        }

        /// <summary>
        /// 擦除/填充用的空单元格：保留当前前景与背景色（BCE 行为），
        /// 但清掉下划线等属性 —— 否则空格会带上 underline，连成一条横线。
        /// tmux / htop 全靠这个把背景色铺满整行。
        /// </summary>
        private Cell EmptyCell()
        {
            Cell cell = new Cell();
            cell.Width = 1;
            cell.Ch = 0;
            cell.Fg = penFg;
            cell.Bg = penBg;
            return cell;
        }

        private static int Clamp(int v, int min, int max)
        {
            if (v < min) return min;
            if (v > max) return max;
            return v;
        }

        private void SetCursor(int row, int col)
        {
            CursorRow = Clamp(row, 0, Rows - 1);
            CursorCol = Clamp(col, 0, Cols - 1);
            wrapPending = false;
        }

        private void MoveCursor(int dRow, int dCol)
        {
            CursorRow = Clamp(CursorRow + dRow, 0, Rows - 1);
            CursorCol = Clamp(CursorCol + dCol, 0, Cols - 1);
        }

        // ================= 光标保存 =================

        private int savedRow, savedCol;
        private int savedFg, savedBg;
        private byte savedFlags;
        private bool savedWrap;

        private void SaveCursor()
        {
            savedRow = CursorRow;
            savedCol = CursorCol;
            savedFg = penFg;
            savedBg = penBg;
            savedFlags = penFlags;
            savedWrap = wrapPending;
        }

        private void RestoreCursor()
        {
            CursorRow = Clamp(savedRow, 0, Rows - 1);
            CursorCol = Clamp(savedCol, 0, Cols - 1);
            penFg = savedFg;
            penBg = savedBg;
            penFlags = savedFlags;
            wrapPending = savedWrap;
        }

        // ================= 输出字符 =================

        private static int CharWidth(uint cp)
        {
            if (cp < 0x20) return 0;
            if (cp < 0x1100) return 1;
            if (cp <= 0x115F) return 2;
            if (cp == 0x2329 || cp == 0x232A) return 2;
            if (cp >= 0x2E80 && cp <= 0xA4CF && cp != 0x303F) return 2;
            if (cp >= 0xAC00 && cp <= 0xD7A3) return 2;
            if (cp >= 0xF900 && cp <= 0xFAFF) return 2;
            if (cp >= 0xFE10 && cp <= 0xFE19) return 2;
            if (cp >= 0xFE30 && cp <= 0xFE6F) return 2;
            if (cp >= 0xFF00 && cp <= 0xFF60) return 2;
            if (cp >= 0xFFE0 && cp <= 0xFFE6) return 2;
            if (cp >= 0x1F300 && cp <= 0x1F64F) return 2;
            if (cp >= 0x1F900 && cp <= 0x1F9FF) return 2;
            if (cp >= 0x20000 && cp <= 0x3FFFD) return 2;
            if (cp >= 0x0300 && cp <= 0x036F) return 0;
            if (cp >= 0xFE00 && cp <= 0xFE0F) return 0;
            return 1;
        }

        private uint lastChar;
        private int lastWidth = 1;

        private void PutChar(uint cp)
        {
            int width = CharWidth(cp);
            if (width == 0)
            {
                // 组合字符, 挂到前一格
                int c0 = CursorCol - 1;
                if (c0 >= 0 && CursorRow < Rows)
                {
                    Screen[CursorRow].Cells[c0].Comb = cp;
                    if (Damaged != null) Damaged();
                }
                return;
            }

            if (wrapPending && AutoWrap)
            {
                CursorCol = 0;
                int oldRow = CursorRow;
                LineFeed();
                // 标记新行是上一行的逻辑续行：复制文本时不该插换行
                if (CursorRow == oldRow + 1 && CursorRow < Rows)
                {
                    Screen[CursorRow].Wrapped = true;
                }
            }
            wrapPending = false;

            if (CursorCol + width > Cols)
            {
                if (AutoWrap)
                {
                    CursorCol = 0;
                    LineFeed();
                }
                else
                {
                    CursorCol = Math.Max(0, Cols - width);
                }
            }

            Line line = CurrentLine();
            if (line == null)
            {
                return;
            }

            if (InsertMode)
            {
                InsertChars(width);
            }

            int col = CursorCol;
            if (col < Cols)
            {
                // 若当前格是宽字符的前半, 先把后半清掉
                ClearWideOverlap(line, col);
                line.Cells[col].Ch = cp;
                line.Cells[col].Comb = 0;
                line.Cells[col].Width = (byte)width;
                line.Cells[col].Flags = penFlags;
                line.Cells[col].Fg = penFg;
                line.Cells[col].Bg = penBg;
                line.Cells[col].LinkId = currentLink;

                if (width == 2 && col + 1 < Cols)
                {
                    ClearWideOverlap(line, col + 1);
                    line.Cells[col + 1].Ch = 0;
                    line.Cells[col + 1].Width = 0;
                    line.Cells[col + 1].Flags = penFlags;
                    line.Cells[col + 1].Fg = penFg;
                    line.Cells[col + 1].Bg = penBg;
                }
            }

            lastChar = cp;
            lastWidth = width;

            CursorCol += width;
            if (CursorCol >= Cols)
            {
                CursorCol = Cols - 1;
                if (AutoWrap)
                {
                    wrapPending = true;
                }
            }

            if (Damaged != null) Damaged();
        }

        private void ClearWideOverlap(Line line, int col)
        {
            // 如果这一格是某个宽字符的第二格, 把前一格也清掉, 避免半个字
            if (col > 0 && line.Cells[col].Width == 0)
            {
                line.Cells[col - 1].Ch = 0;
                line.Cells[col - 1].Width = 1;
            }
            // 如果这一格本身是宽字符的第一格, 清掉它的第二格
            if (col + 1 < Cols && line.Cells[col].Width == 2)
            {
                line.Cells[col + 1].Ch = 0;
                line.Cells[col + 1].Width = 1;
            }
        }

        private void RepeatLast(int count)
        {
            uint cp = lastChar;
            if (cp == 0) return;
            int w = lastWidth;
            for (int i = 0; i < count; i++)
            {
                PutChar(cp);
                if (w == 0) break;
            }
        }

        private Line CurrentLine()
        {
            if (CursorRow < 0 || CursorRow >= Rows) return null;
            return Screen[CursorRow];
        }

        // ================= 滚动 =================

        private void LineFeed()
        {
            if (CursorRow == scrollBot)
            {
                ScrollUp(1);
            }
            else if (CursorRow < Rows - 1)
            {
                CursorRow++;
            }
            // 走到新行就说明这里是"新的一行"，不再是上一行的续行。
            // 自动折行的情况由 PutChar 在那之后重新标记为 true。
            // 不清这个标记的话，回车后的新命令会被当成上一条命令的续行，
            // 重排时两条命令被合并成一条。
            Screen[CursorRow].Wrapped = false;
        }

        private void ReverseIndex()
        {
            if (CursorRow == scrollTop)
            {
                ScrollDown(1);
            }
            else if (CursorRow > 0)
            {
                CursorRow--;
            }
        }

        private void ScrollUp(int count)
        {
            for (int n = 0; n < count; n++)
            {
                Line removed = Screen[scrollTop];
                if (scrollTop == 0 && !usingAlt)
                {
                    PushScrollback(removed);
                }
                for (int r = scrollTop; r < scrollBot; r++)
                {
                    Screen[r] = Screen[r + 1];
                }
                Screen[scrollBot] = NewBlankLine();
            }
        }

        private void ScrollDown(int count)
        {
            for (int n = 0; n < count; n++)
            {
                for (int r = scrollBot; r > scrollTop; r--)
                {
                    Screen[r] = Screen[r - 1];
                }
                Screen[scrollTop] = NewBlankLine();
            }
        }

        private Line NewBlankLine()
        {
            var line = new Line(Cols);
            for (int c = 0; c < Cols; c++)
            {
                line.Cells[c] = EmptyCell();
            }
            return line;
        }

        private void PushScrollback(Line line)
        {
            scrollback.Add(line);
            if (scrollback.Count > ScrollbackLimit)
            {
                scrollback.RemoveAt(0);
            }
        }

        // ================= 编辑操作 =================

        private void InsertLines(int count)
        {
            if (CursorRow < scrollTop || CursorRow > scrollBot) return;
            for (int n = 0; n < count; n++)
            {
                for (int r = scrollBot; r > CursorRow; r--)
                {
                    Screen[r] = Screen[r - 1];
                }
                Screen[CursorRow] = NewBlankLine();
            }
        }

        private void DeleteLines(int count)
        {
            if (CursorRow < scrollTop || CursorRow > scrollBot) return;
            for (int n = 0; n < count; n++)
            {
                for (int r = CursorRow; r < scrollBot; r++)
                {
                    Screen[r] = Screen[r + 1];
                }
                Screen[scrollBot] = NewBlankLine();
            }
        }

        private void InsertChars(int count)
        {
            Line line = CurrentLine();
            if (line == null) return;
            for (int n = 0; n < count; n++)
            {
                for (int c = Cols - 1; c > CursorCol; c--)
                {
                    line.Cells[c] = line.Cells[c - 1];
                }
                line.Cells[CursorCol] = EmptyCell();
            }
        }

        private void DeleteChars(int count)
        {
            Line line = CurrentLine();
            if (line == null) return;
            for (int n = 0; n < count; n++)
            {
                for (int c = CursorCol; c < Cols - 1; c++)
                {
                    line.Cells[c] = line.Cells[c + 1];
                }
                line.Cells[Cols - 1] = EmptyCell();
            }
        }

        private void EraseChars(int count)
        {
            Line line = CurrentLine();
            if (line == null) return;
            for (int c = CursorCol; c < Math.Min(Cols, CursorCol + count); c++)
            {
                line.Cells[c] = EmptyCell();
            }
        }

        private void EraseLine(int mode)
        {
            Line line = CurrentLine();
            if (line == null) return;
            int from = 0, to = Cols - 1;
            if (mode == 0) from = CursorCol;
            else if (mode == 1) to = CursorCol;
            for (int c = from; c <= to && c < Cols; c++)
            {
                line.Cells[c] = EmptyCell();
            }
        }

        private void EraseDisplay(int mode)
        {
            if (mode == 0)
            {
                EraseLine(0);
                for (int r = CursorRow + 1; r < Rows; r++) ClearLine(r);
            }
            else if (mode == 1)
            {
                for (int r = 0; r < CursorRow; r++) ClearLine(r);
                EraseLine(1);
            }
            else
            {
                for (int r = 0; r < Rows; r++) ClearLine(r);
            }
        }

        private void ClearLine(int row)
        {
            Line line = Screen[row];
            // 清整行时连带清掉"变窄后被隐藏"的右半部分，
            // 否则窗口再拉宽时那些旧内容会露出来
            for (int c = 0; c < Line.MaxCols; c++)
            {
                line.Cells[c] = EmptyCell();
            }
        }

        private void DeviceStatus(int mode)
        {
            // 6 = 查询光标位置, 5 = 状态。spike 不回写, 由上层决定。
        }

        // ================= SGR =================

        private void Sgr()
        {
            if (csiArgc == 0)
            {
                penFg = -1; penBg = -1; penFlags = 0;
                return;
            }

            for (int i = 0; i < csiArgc; i++)
            {
                int v = csiArgs[i];
                // 空参数等同 0：ESC[m 是最常见的复位写法（tmux 退出时就用它），
                // 之前把它当"未知参数"忽略，导致绿底之类的状态残留。
                if (v < 0) v = 0;
                switch (v)
                {
                    case 0: penFg = -1; penBg = -1; penFlags = 0; break;
                    case 1: penFlags |= FlagBold; break;
                    case 2: penFlags |= FlagBold; break;
                    case 3: penFlags |= FlagItalic; break;
                    case 4: penFlags |= FlagUnderline; break;
                    case 7: penFlags |= FlagReverse; break;
                    case 8: penFlags |= FlagConceal; break;
                    case 9: penFlags |= FlagStrike; break;
                    case 21: case 22: penFlags = (byte)(penFlags & ~FlagBold); break;
                    case 23: penFlags = (byte)(penFlags & ~FlagItalic); break;
                    case 24: penFlags = (byte)(penFlags & ~FlagUnderline); break;
                    case 27: penFlags = (byte)(penFlags & ~FlagReverse); break;
                    case 28: penFlags = (byte)(penFlags & ~FlagConceal); break;
                    case 29: penFlags = (byte)(penFlags & ~FlagStrike); break;
                    case 39: penFg = -1; break;
                    case 49: penBg = -1; break;
                    case 38:
                    case 48:
                        {
                            bool isFg = v == 38;
                            i = ReadExtendedColor(i, isFg);
                            break;
                        }
                    default:
                        if (v >= 30 && v <= 37) penFg = PaletteColor(v - 30);
                        else if (v >= 40 && v <= 47) penBg = PaletteColor(v - 40);
                        else if (v >= 90 && v <= 97) penFg = PaletteColor(v - 90 + 8);
                        else if (v >= 100 && v <= 107) penBg = PaletteColor(v - 100 + 8);
                        break;
                }
            }
        }

        /// <summary>返回 38;5;n / 38;2;r;g;b 消耗到的下标。</summary>
        private int ReadExtendedColor(int index, bool isFg)
        {
            if (index + 1 >= csiArgc) return index;
            int mode = csiArgs[index + 1];
            if (mode == 5)
            {
                if (index + 2 < csiArgc)
                {
                    int idx = csiArgs[index + 2];
                    if (idx >= 0 && idx <= 255)
                    {
                        if (isFg) penFg = PaletteColor(idx);
                        else penBg = PaletteColor(idx);
                    }
                    return index + 2;
                }
                return index + 1;
            }
            if (mode == 2)
            {
                if (index + 4 < csiArgc)
                {
                    int r = Clamp(csiArgs[index + 2], 0, 255);
                    int g = Clamp(csiArgs[index + 3], 0, 255);
                    int b = Clamp(csiArgs[index + 4], 0, 255);
                    int color = Color.FromArgb(r, g, b).ToArgb();
                    if (isFg) penFg = color; else penBg = color;
                    return index + 4;
                }
                return csiArgc - 1;
            }
            return index + 1;
        }

        private static readonly int[] Base16 =
        {
            unchecked((int)0xFF000000), unchecked((int)0xFFCD0000),
            unchecked((int)0xFF00CD00), unchecked((int)0xFFCDCD00),
            unchecked((int)0xFF0000EE), unchecked((int)0xFFCD00CD),
            unchecked((int)0xFF00CDCD), unchecked((int)0xFFE5E5E5),
            unchecked((int)0xFF7F7F7F), unchecked((int)0xFFFF0000),
            unchecked((int)0xFF00FF00), unchecked((int)0xFFFFFF00),
            unchecked((int)0xFF5C5CFF), unchecked((int)0xFFFF00FF),
            unchecked((int)0xFF00FFFF), unchecked((int)0xFFFFFFFF)
        };

        private static int PaletteColor(int index)
        {
            if (index < 16)
            {
                return Base16[index];
            }
            if (index < 232)
            {
                int i = index - 16;
                int r = i / 36;
                int g = (i / 6) % 6;
                int b = i % 6;
                return Color.FromArgb(Step(r), Step(g), Step(b)).ToArgb();
            }
            int gray = 8 + (index - 232) * 10;
            return Color.FromArgb(gray, gray, gray).ToArgb();
        }

        private static int Step(int v)
        {
            return v == 0 ? 0 : 55 + v * 40;
        }

        // ================= 制表 =================

        private void Tab(int dir)
        {
            if (dir > 0)
            {
                int col = CursorCol + 1;
                while (col < Cols && tabStops[col] == 0) col++;
                CursorCol = Math.Min(Cols - 1, col);
            }
            else
            {
                int col = CursorCol - 1;
                while (col > 0 && tabStops[col] == 0) col--;
                CursorCol = Math.Max(0, col);
            }
        }

        // ================= 供渲染/输入使用 =================

        public string LastLineText()
        {
            var sb = new StringBuilder();
            int row = CursorRow;
            if (row < 0 || row >= Rows) return "";
            Line line = Screen[row];
            for (int c = 0; c < Cols; c++)
            {
                uint ch = line.Cells[c].Ch;
                if (ch == 0) continue;
                if (ch < 0x80) sb.Append((char)ch);
                else sb.Append(char.ConvertFromUtf32((int)ch));
            }
            return sb.ToString();
        }
    }
}
