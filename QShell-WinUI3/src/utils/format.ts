/**
 * 字节数转人类可读文本（GB / MB / KB）。
 * 小于 1MB 时按 KB 展示，避免 45KB 被四舍五入成 "0 MB"。
 */
export function formatMetricBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "--";
  const gb = bytes / 1024 ** 3;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / 1024 ** 2;
  if (mb >= 1) return `${mb.toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/**
 * 命令历史时间：当天显示 HH:mm，跨天显示 M/D HH:mm。
 */
export function formatHistoryTime(ts: number): string {
  const date = new Date(ts);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  return sameDay ? `${hh}:${mm}` : `${date.getMonth() + 1}/${date.getDate()} ${hh}:${mm}`;
}

/**
 * 会话事件时间：HH:mm:ss。
 */
export function formatSessionEventTime(ts: number): string {
  const date = new Date(ts);
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  const ss = String(date.getSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

/**
 * 传输速度格式化（B/s / KB/s / MB/s）。
 */
export function formatSpeed(bps: number): string {
  if (bps < 1024) return `${bps} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(2)} MB/s`;
}

/**
 * 监控图表纵轴速度标签（紧凑单位）。
 */
export function formatAxisSpeed(value: number): string {
  if (value >= 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)}M`;
  if (value >= 1024) return `${(value / 1024).toFixed(0)}K`;
  return `${Math.round(value)}B`;
}
