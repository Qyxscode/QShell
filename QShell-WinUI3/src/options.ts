/**
 * 界面中的静态选项数据（协议、终端类型、波特率、字体）。
 * 抽离后 App.vue 只保留逻辑，避免大段常量挤占可读性。
 */

export interface SelectOption<T = string> {
  label: string;
  value: T;
}

export const protocolOptions: SelectOption[] = [
  { label: "SSH (默认网络终端)", value: "ssh" },
  { label: "Telnet (网络终端)", value: "telnet" },
  { label: "Rlogin (传统 Unix 登录)", value: "rlogin" },
  { label: "Raw TCP (裸 TCP 透传)", value: "raw" },
  { label: "FTP (文件传输)", value: "ftp" },
  { label: "Serial (串口硬件终端)", value: "serial" },
  { label: "Local (本地 PowerShell / CMD)", value: "local" },
];

/** 终端类型：auto 时 SSH 会探测服务器上可用的 terminfo，Telnet 回退 xterm-256color。 */
export const termTypeOptions: SelectOption[] = [
  { label: "自动（推荐）", value: "auto" },
  { label: "xterm-256color", value: "xterm-256color" },
  { label: "xterm", value: "xterm" },
  { label: "vt220", value: "vt220" },
  { label: "vt100", value: "vt100" },
  { label: "linux", value: "linux" },
  { label: "ansi", value: "ansi" },
];

export const serialBaudRateOptions: SelectOption<number>[] = [
  { label: "9600 (标准)", value: 9600 },
  { label: "19200", value: 19200 },
  { label: "38400", value: 38400 },
  { label: "57600", value: 57600 },
  { label: "115200 (高速常用)", value: 115200 },
  { label: "230400", value: 230400 },
  { label: "460800", value: 460800 },
  { label: "921600", value: 921600 },
];

export const presetUiFonts = [
  // 与 App.vue 的 defaultGlobalFont 保持一致：西文 Segoe UI，中文微软雅黑 UI
  { name: "系统推荐默认", value: '"Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", system-ui, -apple-system, "PingFang SC", sans-serif' },
  { name: "微软雅黑 (Microsoft YaHei)", value: '"Microsoft YaHei", sans-serif' },
  { name: "苹方 / 华文细黑 (PingFang SC)", value: '"PingFang SC", "Heiti SC", sans-serif' },
  { name: "Segoe UI", value: '"Segoe UI", sans-serif' },
  { name: "鸿蒙字体 (HarmonyOS Sans SC)", value: '"HarmonyOS Sans SC", "Microsoft YaHei", sans-serif' },
  { name: "OPPO Sans", value: '"OPPO Sans", "Microsoft YaHei", sans-serif' },
  { name: "MiSans (小米兰亭)", value: '"MiSans", "Microsoft YaHei", sans-serif' },
  { name: "思源黑体 (Source Han Sans SC)", value: '"Source Han Sans SC", "Noto Sans SC", sans-serif' },
  { name: "宋体 / 明朝 (SimSun)", value: 'SimSun, "Songti SC", serif' },
  { name: "楷体 (KaiTi)", value: 'KaiTi, "Kaiti SC", serif' },
];
