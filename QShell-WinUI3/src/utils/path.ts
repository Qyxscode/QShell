/**
 * 本地文件名/路径工具（Windows 规则）。
 */

const WINDOWS_RESERVED_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;

/**
 * 只保留"单个文件名"：去掉任何路径分隔符与非法字符，防止 ../ 或绝对路径逃逸出所选目录。
 */
export function sanitizeLocalFileName(raw: string): string {
  const lastSegment = String(raw || "").split(/[/\\]/).pop() || "";
  const stripped = lastSegment
    .replace(/[<>:"|?*\u0000-\u001f]/g, "")
    .replace(/[. ]+$/, "")
    .trim();
  if (!stripped || /^\.+$/.test(stripped)) return "";
  if (WINDOWS_RESERVED_NAME.test(stripped)) return "";
  return stripped;
}

/**
 * 拼接本地目录与文件名。
 */
export function joinLocalPath(dir: string, name: string): string {
  const cleanName = sanitizeLocalFileName(name);
  if (!cleanName) return "";
  return /[\\/]$/.test(dir) ? `${dir}${cleanName}` : `${dir}\\${cleanName}`;
}
