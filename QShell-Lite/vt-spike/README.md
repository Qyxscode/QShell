# QShell Lite — VT 终端核心（spike）

自写终端引擎的可行性验证：**不依赖 PuTTY / xterm**，纯 .NET 4.0 + WinForms + GDI。
覆盖 Windows XP / Win7 / Win10 / Win11。

## 文件

| 文件 | 内容 | 约行数 |
|---|---|---|
| `VtCore.cs` | 解析器 + 屏幕模型（CSI/OSC/SGR、备用屏、滚动区、UTF-8、CJK 双宽、scrollback、超链接） | 1300 |
| `TerminalView.cs` | GDI+ 双缓冲渲染、键盘映射、选择复制、搜索、鼠标上报、IME | 900 |
| `SpikeForm.cs` | SSH.NET 接入、搜索栏、窗口布局 | 500 |
| `lib/Renci.SshNet.dll` | SSH.NET 2020.0.0（最后一个支持 .NET 4.0 的版本，net40） | — |

产物：`VTSpike.exe` 约 40KB + `Renci.SshNet.dll` 804KB（**DLL 必须和 exe 同目录**）。

## 编译

```powershell
$csc = "C:\Windows\Microsoft.NET\Framework\v4.0.30319\csc.exe"
& $csc /nologo /target:winexe /platform:x86 /optimize+ /out:VTSpike.exe `
  /r:System.dll /r:System.Core.dll /r:System.Drawing.dll /r:System.Windows.Forms.dll /r:lib\Renci.SshNet.dll `
  VtCore.cs TerminalView.cs SpikeForm.cs
```

## 已验证

- 颜色：16 色 / 256 色 / 真彩、反显、下划线、删除线
- CJK 双宽对齐（vim 中文注释、框线字符）
- `htop` / `vim` / `tmux` / `less` 正常，退出后屏幕恢复干净
- 备用屏、滚动区、scrollback、窗口 resize 上报（SIGWINCH）
- 选择复制（含自动换行还原）、`Ctrl+F` 搜索、`Ctrl+Shift+V` 粘贴（bracketed paste）
- 方向键 / Home / End / 退格 / F1-F12，应用光标键模式（SS3）跟随
- OSC 8 超链接（Ctrl+点击打开）、鼠标上报（vim/tmux）
- 中文输入（依赖 `WM_CHAR`，`ImeMode.On`）

## 踩过的坑（都是实测，别再踩）

1. **私有 CSI 不能当 SGR**：`ESC[>4;2m`（xterm modifyOtherKeys）被解析成 `4`(下划线)+`2`(粗体)，整屏文字带下划线。凡带 `> = < ?` 前缀的 CSI 一律忽略（`?h/?l` 除外）。
2. **颜色判断不能用 `>= 0`**：ARGB 值（`0xFF...`）是负数，会全部回落默认色。用 `!= -1`。
3. **备用屏必须真的换数组**：只改标志位不换 `Screen`，tmux 退出后整屏残留。
4. **`ESC[m`（空参数）等于复位**：当成未知参数忽略会让状态（如背景色）残留。
5. **BCE**：擦除（EL/ED/ECH/IL/DL）要用**当前背景色**填充，否则 tmux/htop 的色块全丢；但不要带上下划线等属性。
6. **`Line` 要初始化单元格**：`new Cell[]` 默认 `Bg=0`（透明黑）不是 -1，渲染时会被当成"有背景色"。
7. **`\b`（退格）移动光标后要重绘**：readline 左移发的是 `0x08` 不是 `ESC[D`，`Control()` 里不触发 `Damaged` 就会"位置对了但光标不动"。
8. **SS3 序列**：应用光标键模式下方向键是 `ESC O A/B/C/D`，不处理会把 `D` 当普通字符打印。
9. **方向键在 WinForms 会被当导航键**：在 `WndProc` 的 `WM_KEYDOWN` 层直接处理，别指望 `OnKeyDown`。
10. **IME 用 `WM_CHAR`**：`WM_IME_CHAR` 是 ANSI 字符，Win7 微软拼音下会编码错乱（显示成别的字体），交给 `WM_CHAR`（Unicode）即可。
11. **SSH.NET 2020 没有 `ShellStream.ChangeWindowSize`**：反射私有字段 `_channel`（接口是 internal），调 `SendWindowChangeRequest`。
12. **`Wrapped`（续行标记）语义**：`true` 表示"本行是上一行的续行"。重排时如果写成"本行后面还有内容"，一次重排后所有标记全反，第二次拖动就散架。
13. **Resize 复制时不能丢 `Wrapped`**：否则第二次重排认不出续行。
14. **变窄不能删内容**：行按固定宽存储（400 列），`Cols` 只是逻辑宽度。删了内容远端重绘时对不上，表现为残留。
15. **窗口变化的通知要防抖**：bash 每收到一次 SIGWINCH 就重绘当前行，拖动过程中连续发会让它反复重绘、和本地状态错位。改成"拖完 400ms 再发一次"。
16. **reflow 与远端重绘天然冲突**：本地重排会改屏幕和光标，而 readline 也会改，两边对不上。所以默认关闭 reflow（PuTTY 也不做）。

## 已知限制

- 没有脏矩形，全量重绘（htop/vim 刷新率低，够用）
- 鼠标上报只处理点击/拖动/滚轮，未做移动追踪（`?1003`）
- **不做 reflow**：窗口变宽后已折行的内容不会重新拼行（变窄只是隐藏，内容不丢）
- IME 候选框位置未处理（跟随系统默认）
- 未实现：拖选到边缘自动滚动、Shift+点击扩展选择、搜索正则/跨行、DEC 图形字符集、双宽/双高行

## 调试

启动后自动记录到 exe 同目录：

- `vtsplike-raw.bin` —— 远端发来的原始字节
- `vtsplike-tx.bin` —— 我们发出去的字节

状态栏实时显示 `TX: ...` / `RX: ...`（十六进制），排查按键问题非常有效。
`--demo` 看渲染自检画面，`--replay <file>` 回放录制的字节。
