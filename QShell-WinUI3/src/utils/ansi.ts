/**
 * 去除终端输出里的 ANSI 控制序列，用于关键字判断（如密码提示检测）。
 * 只做轻量剥离，不追求完整解析。
 */
export function stripAnsiForCheck(text: string): string {
  return text
    .replace(/\x1b\][^\x07]*\x07/g, "")
    .replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")
    .replace(/\x1b[()][A-Z0-9]/g, "");
}
