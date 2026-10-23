<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick, shallowReactive } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import Icons from "./components/Icons.vue";
import CustomSelect from "./components/CustomSelect.vue";
import ColorPicker from "./components/ColorPicker.vue";
import CustomNumberStepper from "./components/CustomNumberStepper.vue";
import CustomDateTimePicker from "./components/CustomDateTimePicker.vue";
import KeyGenModal from "./components/KeyGenModal.vue";
import UnlockModal from "./components/UnlockModal.vue";
import EncryptionModal from "./components/EncryptionModal.vue";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask, message } from "@tauri-apps/plugin-dialog";

// --- 界面状态 ---
const currentView = ref<"workspace" | "mgmt">("workspace");
const isRightPanelOpen = ref(true);
const rightPanelTab = ref<"sftp" | "memo">("memo");

function onDrawerTransitionEnd() {
  sessions.value.forEach(tab => {
    if (tab.termInstance && tab.fitInstance && tab.alive) {
      try {
        tab.fitInstance.fit();
        sendTerminalResize(tab, tab.termInstance.cols, tab.termInstance.rows);
      } catch (_) {}
    }
  });
}

// 监听右侧栏折叠/展开：在过渡动画的 280ms 期间平滑连续重算终端视口尺寸
watch(isRightPanelOpen, () => {
  const startTs = performance.now();
  const duration = 300;
  const loopFit = (now: number) => {
    sessions.value.forEach(tab => {
      if (tab.termInstance && tab.fitInstance && tab.alive) {
        try {
          tab.fitInstance.fit();
        } catch (_) {}
      }
    });
    if (now - startTs < duration) {
      requestAnimationFrame(loopFit);
    } else {
      onDrawerTransitionEnd();
    }
  };
  requestAnimationFrame(loopFit);
});

// 主机管理大盘：每行卡片数 (1 - 3 列，支持持久化)
const savedMgmtCols = typeof localStorage !== "undefined" ? Number(localStorage.getItem("qshell_mgmt_cols")) || 3 : 3;
const mgmtGridColumns = ref<number>(Math.max(1, Math.min(3, savedMgmtCols)));
function setMgmtGridColumns(cols: number) {
  mgmtGridColumns.value = Math.max(1, Math.min(3, cols));
  try {
    localStorage.setItem("qshell_mgmt_cols", String(mgmtGridColumns.value));
  } catch (e) {}
}

// 计算主机流量百分比 (0 - 100)
function getTrafficProgress(h: HostItem): number {
  if (!h.details || h.details.traffic_total_infinite) return 0;
  const used = parseFloat(h.details.traffic_used || "0");
  const total = parseFloat(h.details.traffic_total || "0");
  if (isNaN(used) || isNaN(total) || total <= 0) return 0;

  // 统一换算为 MB 比较
  const getMultiplier = (unit?: string) => {
    const u = (unit || "GB").toUpperCase();
    if (u.includes("TB")) return 1024 * 1024;
    if (u.includes("GB")) return 1024;
    if (u.includes("MB")) return 1;
    if (u.includes("KB")) return 1 / 1024;
    return 1024;
  };
  const usedMB = used * getMultiplier(h.details.traffic_used_unit);
  const totalMB = total * getMultiplier(h.details.traffic_total_unit);
  const pct = (usedMB / totalMB) * 100;
  return Math.max(0, Math.min(100, pct));
}

// 从 localStorage 读取保存的主题模式，默认为 dark，并立即挂载 data-theme 属性到 document
const savedTheme = (typeof localStorage !== "undefined" && (localStorage.getItem("qshell_theme_mode") as "dark" | "light")) || "dark";
const themeMode = ref<"dark" | "light">(savedTheme);
if (typeof document !== "undefined") {
  document.documentElement.setAttribute("data-theme", savedTheme);
}

// 全局动画动效总开关 (默认开启 true，关闭后强制抑制所有 CSS transition / animation)
const savedAnimations = typeof localStorage !== "undefined" ? localStorage.getItem("qshell_animations_enabled") : null;
const animationsEnabled = ref<boolean>(savedAnimations !== null ? savedAnimations === "true" : true);
function setAnimationsEnabled(val: boolean) {
  animationsEnabled.value = val;
  try {
    localStorage.setItem("qshell_animations_enabled", String(val));
  } catch (e) {}
  if (typeof document !== "undefined") {
    document.documentElement.classList.toggle("disable-animations", !val);
  }
}
if (typeof document !== "undefined" && !animationsEnabled.value) {
  document.documentElement.classList.add("disable-animations");
}

// 左右侧边栏可调节宽度 (原版开阔黄金比例: 左286px, 右280px)
const leftPanelWidth = ref(286);
const rightPanelWidth = ref(280);

// 主机卡片自由拖拽排序
const draggingHostIndex = ref<number | null>(null);
const dragOverHostIndex = ref<number | null>(null);

function handleCardDragStart(index: number, e: DragEvent) {
  draggingHostIndex.value = index;
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(index));
  }
}

function handleCardDragOver(index: number, e: DragEvent) {
  e.preventDefault();
  if (e.dataTransfer) {
    e.dataTransfer.dropEffect = "move";
  }
  if (dragOverHostIndex.value !== index) {
    dragOverHostIndex.value = index;
  }
}

function handleCardDragLeave(index: number) {
  if (dragOverHostIndex.value === index) {
    dragOverHostIndex.value = null;
  }
}

async function handleCardDrop(index: number, e: DragEvent) {
  e.preventDefault();
  if (draggingHostIndex.value !== null && draggingHostIndex.value !== index) {
    const list = [...configData.value.managed_hosts];
    const [moved] = list.splice(draggingHostIndex.value, 1);
    list.splice(index, 0, moved);
    configData.value.managed_hosts = list;
    await invoke("save_hosts_config", { config: configData.value });
  }
  draggingHostIndex.value = null;
  dragOverHostIndex.value = null;
}

function handleCardDragEnd() {
  draggingHostIndex.value = null;
  dragOverHostIndex.value = null;
}

// 终端标签页拖拽排序（指针拖拽，onPointerDragEnd 直接调用 handleTabDrop）
const draggingTabIndex = ref<number | null>(null);
const dragOverTabIndex = ref<number | null>(null);

function handleTabDrop(index: number, e: DragEvent) {
  e.preventDefault();
  if (draggingTabIndex.value !== null && draggingTabIndex.value !== index) {
    const list = [...sessions.value];
    const [moved] = list.splice(draggingTabIndex.value, 1);
    list.splice(index, 0, moved);
    sessions.value = list;
  }
  draggingTabIndex.value = null;
  dragOverTabIndex.value = null;
}
const leftMonitorCardHeight = ref(265); // 初始监控卡片高度 (紧凑去留白，精致黄金尺寸)
let isDraggingLeftSplitter = false;
let isDraggingRightSplitter = false;
let isDraggingLeftVerticalSplitter = false;

function onLeftSplitterDown(e: MouseEvent) {
  isDraggingLeftSplitter = true;
  document.addEventListener("mousemove", onSplitterMove);
  document.addEventListener("mouseup", onSplitterUp);
  e.preventDefault();
}

function onLeftVerticalSplitterDown(e: MouseEvent) {
  isDraggingLeftVerticalSplitter = true;
  document.addEventListener("mousemove", onSplitterMove);
  document.addEventListener("mouseup", onSplitterUp);
  e.preventDefault();
}

function onRightSplitterDown(e: MouseEvent) {
  isDraggingRightSplitter = true;
  document.addEventListener("mousemove", onSplitterMove);
  document.addEventListener("mouseup", onSplitterUp);
  e.preventDefault();
}

function onSplitterMove(e: MouseEvent) {
  if (isDraggingLeftSplitter) {
    const newWidth = e.clientX - 12;
    if (newWidth >= 260 && newWidth <= 480) {
      leftPanelWidth.value = newWidth;
    }
  } else if (isDraggingLeftVerticalSplitter) {
    // 垂直拖拽：紧凑留白后卡片自然内容高度约 258px，最小高度锁定 260px，既极致紧凑又绝不裁切
    const newHeight = e.clientY - 44 - 10;
    if (newHeight >= 260 && newHeight <= 480) {
      leftMonitorCardHeight.value = newHeight;
    }
  } else if (isDraggingRightSplitter) {
    const newWidth = window.innerWidth - e.clientX - 12;
    if (newWidth >= 250 && newWidth <= 520) {
      rightPanelWidth.value = newWidth;
    }
  }
}

function onSplitterUp() {
  isDraggingLeftSplitter = false;
  isDraggingLeftVerticalSplitter = false;
  isDraggingRightSplitter = false;
  isDraggingSftpCol = null;
  document.removeEventListener("mousemove", onSplitterMove);
  document.removeEventListener("mouseup", onSplitterUp);
}

// SFTP 列宽拖拽调节状态与默认列宽 (名称列默认收短1/3左右, 赋予更多紧凑展示空间)
const sftpColWidths = ref({
  name: 120, // 初始默认由 180px 缩短 1/3 至 120px
  size: 55,
  perm: 55
});
let isDraggingSftpCol: 'name' | 'size' | null = null;
let sftpDragStartX = 0;
let sftpDragStartWidth = 0;

function onSftpColResizeDown(col: 'name' | 'size', e: MouseEvent) {
  isDraggingSftpCol = col;
  sftpDragStartX = e.clientX;
  sftpDragStartWidth = sftpColWidths.value[col];
  document.addEventListener("mousemove", onSftpColResizeMove);
  document.addEventListener("mouseup", onSftpColResizeUp);
  e.preventDefault();
  e.stopPropagation();
}

function onSftpColResizeMove(e: MouseEvent) {
  if (!isDraggingSftpCol) return;
  const delta = e.clientX - sftpDragStartX;
  if (isDraggingSftpCol === 'name') {
    sftpColWidths.value.name = Math.max(50, Math.min(350, sftpDragStartWidth + delta));
  } else if (isDraggingSftpCol === 'size') {
    sftpColWidths.value.size = Math.max(40, Math.min(150, sftpDragStartWidth + delta));
  }
}

function onSftpColResizeUp() {
  isDraggingSftpCol = null;
  document.removeEventListener("mousemove", onSftpColResizeMove);
  document.removeEventListener("mouseup", onSftpColResizeUp);
}

function hexToRgb(hex: string): [number, number, number] | null {
  const value = hex.trim().replace("#", "");
  if (!/^[0-9a-fA-F]{6}$/.test(value)) return null;
  return [
    parseInt(value.slice(0, 2), 16),
    parseInt(value.slice(2, 4), 16),
    parseInt(value.slice(4, 6), 16),
  ];
}

function relativeLuminance(hex: string): number {
  const rgb = hexToRgb(hex);
  if (!rgb) return 0;
  const [r, g, b] = rgb.map((channel) => {
    const c = channel / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrastRatio(foreground: string, background: string): number {
  const fg = relativeLuminance(foreground);
  const bg = relativeLuminance(background);
  const lighter = Math.max(fg, bg);
  const darker = Math.min(fg, bg);
  return (lighter + 0.05) / (darker + 0.05);
}

// 保存设置、新建终端时也要保证可读性，不能只在切换主题时兜底，
// 否则"日间模式直接选白色并保存"会留下不可读的白底白字。
function ensureReadableFg(fg: string, bg: string, fallback: string): string {
  if (!fg) return fallback;
  return contrastRatio(fg, bg) < 4.5 ? fallback : fg;
}

function executeThemeChange() {
  const newTheme = themeMode.value === "dark" ? "light" : "dark";
  themeMode.value = newTheme;
  document.documentElement.setAttribute("data-theme", newTheme);
  try {
    localStorage.setItem("qshell_theme_mode", newTheme);
  } catch (e) {}

  // 终端默认跟随全局日夜模式联动：
  const isDark = newTheme === "dark";
  const darkPreset = "#81C995";
  const lightPreset = "#111318";
  const prevMode = terminalSettings.value.term_bg_mode || "dark";
  const prevFg = (terminalSettings.value.terminal_fg_color || "").trim().toLowerCase();
  const presetPrev = (prevMode === "dark" ? darkPreset : lightPreset).toLowerCase();
  const targetBg = isDark ? "#0A0B0E" : "#F4F5F7";
  const fallbackFg = isDark ? darkPreset : lightPreset;

  terminalSettings.value.term_bg_mode = isDark ? "dark" : "light";
  terminalSettings.value.terminal_bg_color = targetBg;

  // 预设色或对比度不足时自动兜底，避免白底白字 / 黑底黑字。
  // 自定义背景图优先根据图片亮度和遮罩自动选字色。
  if (terminalBgImageData.value && terminalSettings.value.bg_image_auto_fg) {
    const effective = terminalBgImageLuminance.value * (1 - terminalSettings.value.bg_image_dim);
    terminalSettings.value.terminal_fg_color = effective < 0.45 ? "#FFFFFF" : "#111318";
  } else if (!prevFg || prevFg === presetPrev || contrastRatio(prevFg, targetBg) < 4.5) {
    terminalSettings.value.terminal_fg_color = fallbackFg;
  }

  applyTerminalSettings(false); // 同步终端外观，但不关闭偏好弹窗
}

// 现代化圆形扩散亮暗切换动画 (从右下角/点击按钮源向全屏水波涟漪扩大)
let themeTransitionToken = 0;
function toggleTheme(event?: MouseEvent) {
  // 全局动画关闭或系统要求减少动态效果时，直接切换，不创建 View Transition。
  const prefersReducedMotion =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  if (!animationsEnabled.value || prefersReducedMotion || !document.startViewTransition) {
    executeThemeChange();
    return;
  }

  // View Transition 已经会生成整页快照；切换期间暂停其他装饰动画，
  // 避免 LED、卡片阴影和 CSS transition 同时抢合成线程。
  const token = ++themeTransitionToken;
  document.documentElement.classList.add("theme-switching");
  const cleanup = () => {
    if (token !== themeTransitionToken) return;
    clearTimeout(cleanupTimer);
    document.documentElement.classList.remove("theme-switching");
  };
  const cleanupTimer = window.setTimeout(cleanup, 1200);

  // 计算扩散圆心：优先取点击事件坐标，若无则默认取屏幕右下角
  const x = event ? event.clientX : window.innerWidth;
  const y = event ? event.clientY : window.innerHeight;
  const endRadius = Math.hypot(
    Math.max(x, window.innerWidth - x),
    Math.max(y, window.innerHeight - y)
  );

  const transition = document.startViewTransition(() => {
    executeThemeChange();
  });

  transition.ready.then(() => {
    const clipPath = [
      `circle(0px at ${x}px ${y}px)`,
      `circle(${endRadius}px at ${x}px ${y}px)`
    ];
    document.documentElement.animate(
      {
        clipPath: clipPath,
      },
      {
        duration: 700,
        easing: "cubic-bezier(0.25, 1, 0.5, 1)",
        pseudoElement: "::view-transition-new(root)",
      }
    );
  });
  transition.finished.finally(cleanup);
}

// --- 窗口控制 (兼容浏览器直接运行与 Tauri 原生环境) ---
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
// telnet / serial / local 都走 GenericStreamManager，只有 SSH 走 ssh.rs
function isGenericProtocol(protocol?: string): boolean {
  return protocol === "telnet" || protocol === "serial" || protocol === "local";
}
const appWindow = isTauri ? getCurrentWindow() : null;
// 版本号从 tauri.conf.json 读取，避免每次升级都要改前端硬编码。
const appVersion = ref("0.99.71");
if (isTauri) {
  getVersion()
    .then((version) => { appVersion.value = version; })
    .catch(() => { /* 浏览器预览或读取失败时保留兜底版本号 */ });
}
let titleBarDoubleClickToggleAt = 0;

function startDragging(e: MouseEvent) {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest("button, input, select, textarea, .context-menu, .modal-card")) return;
  // 自定义拖拽会吞掉系统标题栏的双击最大化，这里手动处理第二次按下。
  if (e.detail === 2) {
    toggleMaximizeWindow();
    titleBarDoubleClickToggleAt = performance.now();
    return;
  }
  if (appWindow) appWindow.startDragging();
}

function onTitleBarDoubleClick(e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  if (target?.closest("button, input, select, textarea, .context-menu, .modal-card")) return;
  // mousedown(detail=2) 已经处理过时，避免 dblclick 再次切换。
  if (performance.now() - titleBarDoubleClickToggleAt < 400) return;
  toggleMaximizeWindow();
}
function minimizeWindow() { appWindow?.minimize(); }
function toggleMaximizeWindow() { appWindow?.toggleMaximize(); }
function closeWindow() { appWindow?.close(); }

// --- 数据模型 ---
interface HostDetails {
  memo?: string;
  expire_date?: string;
  alert_expire_days?: number;
  alert_traffic_pct?: number;
  price?: string;
  currency?: string;
  traffic_total?: string;
  traffic_total_unit?: string;
  traffic_total_infinite?: boolean;
  traffic_used?: string;
  traffic_used_unit?: string;
  reset_day?: number;
  reset_hour?: number;
  reset_min?: number;
  reset_sec?: number;
  traffic_direction?: string;
  target_interface?: string;
  traffic_price?: string;
  traffic_currency?: string;
  traffic_cycle_used_bytes?: number;
  traffic_source?: string;
  traffic_custom_command?: string;
  traffic_source_effective?: string;
  traffic_source_error?: string;
  traffic_last_sync_ts?: number;
  traffic_delta_bytes?: number;
  last_raw_bytes?: number;
  traffic_counter_sig?: string;
  traffic_boot_id?: string;
  last_reset_timestamp?: number;
  traffic_manual_override?: boolean;
  traffic_iface_baseline?: string;
  traffic_vnstat_anchor_bytes?: number;
  traffic_vnstat_anchor_ts?: number;
  traffic_cycle_anchor_used_bytes?: number;
  traffic_force_nic?: boolean;
}

interface TrafficAuditResult {
  traffic_cycle_used_bytes: number;
  traffic_delta_bytes: number;
  traffic_used: string;
  traffic_used_unit: string;
  last_raw_bytes: number;
  traffic_counter_sig: string;
  traffic_boot_id: string;
  last_reset_timestamp: number;
  traffic_source_effective: string;
  traffic_source_error: string;
  traffic_last_sync_ts: number;
  traffic_iface_baseline: string;
}

interface HostItem {
  name: string;
  protocol?: "ssh" | "telnet" | "serial" | "local";
  host: string;
  port: number;
  user: string;
  pwd?: string;
  key_path?: string;
  passphrase?: string;
  serial_data_bits?: number;
  serial_parity?: "none" | "odd" | "even";
  serial_stop_bits?: number;
  serial_flow_control?: "none" | "software" | "hardware";
  serial_newline?: "cr" | "lf" | "crlf";
  serial_local_echo?: boolean;
  serial_dtr?: boolean;
  serial_rts?: boolean;
  local_shell?: "powershell" | "cmd";
  term_type?: string;
  use_agent?: boolean;
  details?: HostDetails;
  latency_text?: string;
}

interface DeviceGroup {
  group: string;
  hosts: HostItem[];
  collapsed?: boolean;
}

interface MemoItem {
  name: string;
  cmd: string;
}

interface MemoGroup {
  group: string;
  items: MemoItem[];
  collapsed?: boolean;
}

interface AppConfig {
  version?: string;
  devices: DeviceGroup[];
  managed_hosts: HostItem[];
  memos?: MemoGroup[];
  settings?: any;
}

const configData = ref<AppConfig>({ devices: [], managed_hosts: [], memos: [] });

// --- 会话与多标签页体系 ---
interface SessionTab {
  id: string;
  name: string;
  host: HostItem;
  termInstance?: Terminal;
  fitInstance?: FitAddon;
  resizeObserver?: ResizeObserver;
  unlisten?: UnlistenFn;
  alive: boolean;
  is_connecting?: boolean;
  is_locked?: boolean;
  latency?: number;
  read_only?: boolean;
  net_if?: string;
  runtimePassword?: string;
}

const sessions = ref<SessionTab[]>([]);
const activeSessionId = ref<string>("");
const activeSession = computed(() => sessions.value.find(s => s.id === activeSessionId.value));

function toggleTabLock(tab: SessionTab) {
  tab.is_locked = !tab.is_locked;
}

// --- 系统实时监控 (未连接时默认为 0，连接后随活动终端实时采样) ---
const monitorStats = ref({
  cpu: 0,
  mem: 0,
  disk: 0,
  net_up: "0 KB/s",
  net_down: "0 KB/s",
  net_if: "",
  net_if_options: [] as string[],
  latency: 0,
  cpu_model: "",
  cpu_cores: 0,
  mem_total_bytes: 0,
  mem_used_bytes: 0,
  disk_total_bytes: 0,
  disk_used_bytes: 0,
});
const sessionMonitorStats = new Map<string, typeof monitorStats.value>();

type NetworkViewMode = "live" | "stats";
const networkView = ref<NetworkViewMode>("live");
interface NetworkSample { ts: number; up: number; down: number; }
const networkSamples = ref<NetworkSample[]>([]);

function parseNetworkSpeed(value: string): number {
  const match = String(value || "").match(/([\d.]+)\s*(B|KB|MB|GB)\/s/i);
  if (!match) return 0;
  const amount = Number(match[1]) || 0;
  const unit = match[2].toUpperCase();
  const multiplier = unit === "GB" ? 1024 ** 3 : unit === "MB" ? 1024 ** 2 : unit === "KB" ? 1024 : 1;
  return amount * multiplier;
}

function resetNetworkHistory() {
  networkSamples.value = [];
}

function recordNetworkSample(upText: string, downText: string) {
  const now = Date.now();
  const upBps = parseNetworkSpeed(upText);
  const downBps = parseNetworkSpeed(downText);
  const samples = networkSamples.value;
  const last = samples[samples.length - 1];
  if (!last) {
    samples.push({ ts: now, up: 0, down: 0 });
    return;
  }
  const dt = Math.min(Math.max((now - last.ts) / 1000, 0), 10);
  samples.push({
    ts: now,
    up: last.up + upBps * dt,
    down: last.down + downBps * dt,
  });
  const cutoff = now - 65 * 60 * 1000;
  while (samples.length > 2 && samples[0].ts < cutoff) samples.shift();
}

const showNetIfPicker = ref(false);

// 统计视图：把采样累计值差分回"瞬时速率"，画成带刻度的折线图。
const networkRateSeries = computed(() => {
  const samples = networkSamples.value;
  const out: Array<{ up: number; down: number }> = [];
  for (let i = 1; i < samples.length; i++) {
    const dt = Math.max(0.001, (samples[i].ts - samples[i - 1].ts) / 1000);
    out.push({
      up: Math.max(0, (samples[i].up - samples[i - 1].up) / dt),
      down: Math.max(0, (samples[i].down - samples[i - 1].down) / dt),
    });
  }
  return out.slice(-60);
});

const networkRateMax = computed(() => {
  let max = 0;
  for (const point of networkRateSeries.value) {
    max = Math.max(max, point.up, point.down);
  }
  return max > 0 ? max : 1;
});

function buildNetworkPolyline(key: "up" | "down"): string {
  const series = networkRateSeries.value;
  if (series.length < 2) return "";
  const max = networkRateMax.value;
  const x0 = 28;
  const x1 = 224;
  const baseline = 60;
  const plotHeight = 48;
  return series
    .map((point, index) => {
      const x = x0 + (index / (series.length - 1)) * (x1 - x0);
      const y = baseline - (point[key] / max) * plotHeight;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function formatAxisSpeed(value: number): string {
  if (value >= 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)}M`;
  if (value >= 1024) return `${(value / 1024).toFixed(0)}K`;
  return `${Math.round(value)}B`;
}
let refreshMetricsNow: (() => void) | null = null;
const availableNetInterfaces = computed(() =>
  monitorStats.value.net_if_options.filter((name) => name !== "lo")
);

function selectNetInterface(name: string) {
  const tab = activeSession.value;
  if (tab) tab.net_if = name;
  showNetIfPicker.value = false;
  monitorStats.value.net_if = name;
  monitorStats.value.net_up = "0 KB/s";
  monitorStats.value.net_down = "0 KB/s";
  refreshMetricsNow?.();
}

const LED_COLUMNS = 18;
const LED_ROWS = 10;
const TOTAL_LEDS = LED_COLUMNS * LED_ROWS; // 18 列 x 10 行，灯珠为正方形

// 亮起区域按"离头部越远越暗"做整体渐变，而不是只有头部 4 颗才有颜色差异。
function ledFadeOpacity(index: number, activeCount: number): number | undefined {
  if (activeCount <= 0 || index > activeCount) return undefined;
  const distance = Math.max(0, activeCount - index);
  return Number(Math.max(0.45, 1 - distance * 0.035).toFixed(2));
}

function calculateLedActiveCount(speedStr: string): number {
  if (!speedStr || sessions.value.length === 0) return 0;
  const s = speedStr.trim().toUpperCase();
  const m = s.match(/([0-9]+(?:\.[0-9]+)?)\s*([A-Z\/]+)/);
  if (!m) return 0;
  const val = parseFloat(m[1]);
  const unit = m[2];
  if (val <= 0) return 0;

  // 统一换算为 KB/s
  let kbps = 0;
  if (unit.includes("GB")) {
    kbps = val * 1024.0 * 1024.0;
  } else if (unit.includes("MB")) {
    kbps = val * 1024.0;
  } else if (unit.includes("KB")) {
    kbps = val;
  } else {
    kbps = val / 1024.0;
  }

  // 科学真实网络刻度（共 180 颗灯珠，每排 18 颗）：
  // - 10 KB/s 以下（心跳/闲置）：1 ~ 2 颗
  // - 100 KB/s：约 6 颗
  // - 1 MB/s (1024 KB/s)：18 颗（刚好第 1 排）
  // - 5 MB/s：约 53 颗
  // - 15 MB/s（百兆宽带满载）：约 76 颗
  // - 50 MB/s：约 124 颗
  // - 100 MB/s（千兆跑满）：180 颗（10 排全满）
  let count = 0;
  if (kbps < 1.0) {
    count = 1;
  } else if (kbps <= 1024.0) {
    const logRatio = Math.log10(kbps) / Math.log10(1024.0);
    count = Math.max(1, Math.round(1 + logRatio * (LED_COLUMNS - 1)));
  } else if (kbps <= 102400.0) {
    const logRatio = Math.log10(kbps / 1024.0) / Math.log10(100.0);
    count = Math.round(LED_COLUMNS + logRatio * (TOTAL_LEDS - LED_COLUMNS));
  } else {
    count = TOTAL_LEDS;
  }
  return Math.max(1, Math.min(TOTAL_LEDS, count));
}

const activeUpLedCount = computed(() => calculateLedActiveCount(monitorStats.value.net_up));
const activeDownLedCount = computed(() => calculateLedActiveCount(monitorStats.value.net_down));

// 终端连接与延迟指示器状态色判断
// 离线/未连接=红色；低于150ms=绿色；150ms~250ms=黄色；高于250ms=橙色；连接中/刚连接测速中=浅蓝
const latencyStatusClass = computed(() => {
  const sess = activeSession.value;
  if (!sess) return 'status-offline';

  // 延迟值优先取 monitorStats.latency，若切标签尚未出结果则回退到该标签已缓存的 latency
  const lat = monitorStats.value.latency > 0 ? monitorStats.value.latency : (sess.latency || 0);

  // 1. 如果已有有效测速值（说明主机连通，绝对不能显示为离线红色！）
  if (lat > 0) {
    if (lat < 150) return 'status-green';       // < 150ms: 绿色
    if (lat <= 250) return 'status-yellow';     // 150ms ~ 250ms: 黄色
    return 'status-orange';                    // > 250ms: 橙色
  }

  // 2. 处于连接中，或已连上存活但首个 ping 仍在回包中 -> 浅蓝测速中
  if (sess.is_connecting || sess.alive) {
    return 'status-measuring';
  }

  // 3. 既无测速结果、也未处于连接/存活状态 -> 离线红色
  return 'status-offline';
});

// 指示器展示文字计算属性
const latencyDisplayText = computed(() => {
  const sess = activeSession.value;
  if (!sess) return '离线';
  const lat = monitorStats.value.latency > 0 ? monitorStats.value.latency : (sess.latency || 0);
  if (lat > 0) return `在线 ${lat} ms`;
  if (sess.is_connecting || sess.alive) return '测速中';
  return '离线';
});

// 方案 A：流光头部始终停在速度对应的最后一颗亮灯处，不再循环扫描。
const upRunnerIdx = ref(0);
const downRunnerIdx = ref(0);

// 计算灯珠在彗星流光中的身份：0=头部高亮, 1=尾焰第1级, 2=尾焰第2级, 3=尾焰第3级, -1=普通背景
function getRunnerLevel(idx: number, runnerHead: number, maxCount: number): number {
  if (runnerHead <= 0 || maxCount <= 0 || idx > maxCount) return -1;
  const diff = runnerHead - idx;
  if (diff >= 0 && diff <= 3) return diff;
  return -1;
}

watch(activeUpLedCount, (count) => {
  upRunnerIdx.value = count;
}, { immediate: true });

watch(activeDownLedCount, (count) => {
  downRunnerIdx.value = count;
}, { immediate: true });

// 统一终端窗口尺寸调整适配 (支持 SSH 及 Telnet NAWS 协议动态协商)
function sendTerminalResize(tab: SessionTab, cols: number, rows: number) {
  if (!tab.alive || cols <= 0 || rows <= 0) return;
  if (isGenericProtocol(tab.host?.protocol)) {
    invoke("resize_generic_session", { sessionId: tab.id, cols, rows }).catch(() => {});
  } else {
    invoke("resize_ssh_session", { sessionId: tab.id, cols, rows }).catch(() => {});
  }
}
watch(currentView, (val) => {
  if (val === "workspace") {
    nextTick(() => {
      // 延时双重触发，保证 CSS display: flex 容器完成几何尺寸恢复
      const refitTerminals = () => {
        sessions.value.forEach(tab => {
          if (tab.termInstance && tab.fitInstance && tab.alive) {
            try {
              tab.fitInstance.fit();
              sendTerminalResize(tab, tab.termInstance.cols, tab.termInstance.rows);
              // 触发 xterm 缓冲区刷新重绘
              tab.termInstance.refresh(0, tab.termInstance.rows - 1);
            } catch (_) {}
          }
        });
        const activeTab = sessions.value.find(s => s.id === activeSessionId.value);
        if (activeTab?.termInstance) {
          try {
            activeTab.termInstance.focus();
          } catch (_) {}
        }
      };

      refitTerminals();
      setTimeout(refitTerminals, 60);
      setTimeout(refitTerminals, 180);
    });
  }
});

let monitorInterval: number | null = null;
// 系统指标轮询的 in-flight 标记：后端一旦卡住，3 秒一次的轮询会把
// blocking 线程池逐步抽干，最终拖死 SFTP 传输与流量审计。
let metricsPollInFlight = false;

function formatMetricBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "--";
  const gb = bytes / (1024 ** 3);
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / (1024 ** 2);
  // 小于 1MB 时按 KB 展示，否则 45KB 会被四舍五入成 "0 MB"。
  if (mb >= 1) return `${mb.toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function resetMonitorStats() {
  refreshMetricsNow = null;
  if (latencyInterval) { clearInterval(latencyInterval); latencyInterval = null; }
  monitorStats.value = {
    cpu: 0,
    mem: 0,
    disk: 0,
    net_up: "0 KB/s",
    net_down: "0 KB/s",
    net_if: "",
    net_if_options: [],
    latency: 0,
    cpu_model: "",
    cpu_cores: 0,
    mem_total_bytes: 0,
    mem_used_bytes: 0,
    disk_total_bytes: 0,
    disk_used_bytes: 0,
  };
  upRunnerIdx.value = 0;
  downRunnerIdx.value = 0;
  resetNetworkHistory();
}

// --- SFTP 数据模型与状态 ---
interface SftpFileItem {
  name: string;
  is_dir: boolean;
  size: string;
  size_bytes: number;
  mode_str: string;
  mode_oct: string;
  mtime: number;
}

interface LocalFileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
}

interface TransferTask {
  task_id: string;
  filename: string;
  is_upload: boolean;
  transferred: number;
  total: number;
  speed_bps: number;
  status: "running" | "completed" | "failed" | "cancelled";
  error?: string;
}

// 拖拽批量上传/多选下载会在同一毫秒发起多个任务；只用 Date.now() 会得到相同 taskId，
// 后端会因此共用同一个 .qshell-tmp-<taskId> 临时文件，前端 v-for 的 key 也会冲突。
let transferTaskSeq = 0;
function makeTaskId(prefix: "up" | "down"): string {
  transferTaskSeq = (transferTaskSeq + 1) % 1_000_000;
  return `${prefix}-${Date.now()}-${transferTaskSeq}`;
}

const currentSftpPath = ref("/");
const sftpFiles = ref<SftpFileItem[]>([]);
const sftpSortKey = ref<"name" | "size">("name");
const sftpSortAsc = ref(true);
const selectedSftpFile = ref<SftpFileItem | null>(null);
const selectedSftpFiles = ref<SftpFileItem[]>([]);
let sftpSelectionAnchorIndex = -1;
let sftpRequestSeq = 0;
const isSftpLoading = ref(false);

const showLocalFilePicker = ref(false);
const localPickerPath = ref("");
const localPickerEntries = ref<LocalFileEntry[]>([]);
const localPickerRoots = ref<LocalFileEntry[]>([]);
const localKnownFolders = ref<Array<{ id: string; name: string; path: string; icon: string }>>([]);
const localPickerMode = ref<"files" | "drives" | "network">("files");
const localPickerAction = ref<"open" | "save">("open");
const localPickerTitle = ref("选择文件");
const localPickerFileName = ref("");
const localPickerSelected = ref<LocalFileEntry | null>(null);
const localPickerLoading = ref(false);
const localPickerError = ref("");
const localPickerThumbnails = ref<Record<string, string>>({});
const LOCAL_THUMB_CACHE_LIMIT = 300;
let localPickerDirToken = 0;
let localPickerResolve: ((path: string | null) => void) | null = null;
const activeTransfers = ref<TransferTask[]>([]);
const showTransferMonitor = ref(false);

// --- 搜索过滤 ---
const searchKeyword = ref("");

// --- 主机到期与超额流量预警机制 ---
interface HostAlertInfo {
  hostName: string;
  type: "expire" | "traffic";
  msg: string;
  hostItem?: HostItem;
}

const hostAlertList = ref<HostAlertInfo[]>([]);
const showAlertPopover = ref(false);

const alertPopoverTargetRight = ref(120);
const transferPopoverTargetRight = ref(85);

function toggleAlertPopover(e?: MouseEvent) {
  if (showTransferMonitor.value) {
    showTransferMonitor.value = false;
  }
  if (!showAlertPopover.value && e?.currentTarget) {
    const btn = e.currentTarget as HTMLElement;
    const rect = btn.getBoundingClientRect();
    const btnCenterX = rect.left + rect.width / 2;
    // 气泡固定 right: 15px，气泡宽度 360px
    // 气泡右边界距离窗口右侧为 15px，所以小三角相对于气泡右边界的距离为：
    const popoverRightEdge = window.innerWidth - 15;
    alertPopoverTargetRight.value = Math.max(16, Math.min(340, popoverRightEdge - btnCenterX));
  }
  showAlertPopover.value = !showAlertPopover.value;
}

function toggleTransferMonitor(e?: MouseEvent) {
  if (showAlertPopover.value) {
    showAlertPopover.value = false;
  }
  if (!showTransferMonitor.value && e?.currentTarget) {
    const btn = e.currentTarget as HTMLElement;
    const rect = btn.getBoundingClientRect();
    const btnCenterX = rect.left + rect.width / 2;
    // 气泡固定 right: 15px，气泡宽度 320px
    const popoverRightEdge = window.innerWidth - 15;
    transferPopoverTargetRight.value = Math.max(16, Math.min(300, popoverRightEdge - btnCenterX));
  }
  showTransferMonitor.value = !showTransferMonitor.value;
}

function handleGlobalClick(e: MouseEvent) {
  closeContextMenu();
  const target = e.target as HTMLElement | null;
  if (!target) return;

  // 点击波特率选择器外部时，自动收起波特率下拉框
  if (showBaudDropdown.value) {
    if (!target.closest(".combo-baud-wrapper")) {
      showBaudDropdown.value = false;
    }
  }

  // 点击非通知气泡及非通知按钮区域时，自动收起通知气泡
  if (showAlertPopover.value) {
    if (!target.closest(".alert-popover") && !target.closest(".btn-toggle-alert")) {
      showAlertPopover.value = false;
    }
  }

  // 点击非传输气泡及非传输按钮区域时，自动收起传输气泡
  if (showTransferMonitor.value) {
    if (!target.closest(".transfer-monitor-popover:not(.alert-popover)") && !target.closest(".btn-toggle-transfer")) {
      showTransferMonitor.value = false;
    }
  }
}

function clearAllAlerts() {
  hostAlertList.value = [];
  showAlertPopover.value = false;
}

function clearTransferHistory() {
  // 只清理已结束的记录，正在运行的任务必须保留
  activeTransfers.value = activeTransfers.value.filter(t => t.status === 'running');
}

async function cancelTransfer(taskId: string) {
  try {
    await invoke("cancel_sftp_transfer", { taskId });
  } catch (e) {
    console.warn("[sftp] 取消传输失败:", e);
  }
}

const highlightedHostKey = ref<string | null>(null);
let highlightTimer: any = null;

function handleAlertClick(al: HostAlertInfo) {
  showAlertPopover.value = false;
  if (al.hostItem) {
    currentView.value = "mgmt";
    // 聚焦并高亮指定主机卡片
    const key = `${al.hostItem.host}:${al.hostItem.port}`;
    highlightedHostKey.value = key;
    if (highlightTimer) clearTimeout(highlightTimer);

    // 延时滚动到可视区域
    setTimeout(() => {
      const el = document.getElementById(`host-card-${al.hostItem?.host}-${al.hostItem?.port}`);
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
      }
    }, 50);

    // 平缓呼吸 3 次，每次 1.6 秒，总共 4.8 秒后自动移除类
    highlightTimer = setTimeout(() => {
      if (highlightedHostKey.value === key) {
        highlightedHostKey.value = null;
      }
    }, 4800);
  }
}

function evaluateHostAlerts() {
  const alerts: HostAlertInfo[] = [];
  const now = new Date();
  configData.value.managed_hosts.forEach(h => {
    // 1. 到期检测
    if (h.details?.expire_date && h.details.expire_date !== "无限") {
      const expDate = new Date(h.details.expire_date.replace(/-/g, "/"));
      if (!isNaN(expDate.getTime())) {
        const diffDays = Math.ceil((expDate.getTime() - now.getTime()) / (1000 * 3600 * 24));
        if (diffDays <= 3) {
          alerts.push({
            hostName: h.name,
            type: "expire",
            msg: diffDays <= 0 ? `已于 ${h.details.expire_date} 到期！` : `将在 ${diffDays} 天后 (${h.details.expire_date}) 到期`,
            hostItem: h,
          });
        }
      }
    }
    // 2. 超额流量检测
    if (h.details && !h.details.traffic_total_infinite && h.details.traffic_total && h.details.traffic_used) {
      // 用统一换算后的百分比，避免 used/total 单位不同时算出 500% 之类的假报警。
      const pct = getTrafficProgress(h);
      const threshold = typeof h.details.alert_traffic_pct === "number" && h.details.alert_traffic_pct > 0
        ? h.details.alert_traffic_pct
        : 90;
      if (pct >= threshold) {
        alerts.push({
          hostName: h.name,
          type: "traffic",
          msg: `流量使用率达 ${Math.round(pct)}%（阈值 ${threshold}%）`,
          hostItem: h,
        });
      }
    }
  });
  hostAlertList.value = alerts;
}

// --- 全局右键菜单系统 ---
const contextMenuRef = ref<HTMLElement | null>(null);
const contextMenu = ref({
  show: false,
  x: 0,
  y: 0,
  targetType: "" as "host" | "group" | "tab" | "terminal" | "sftp_item" | "sftp_bg" | "memo_item" | "memo_group" | "memo_bg" | "",
  targetHost: null as HostItem | null,
  targetGroup: null as DeviceGroup | null,
  targetTab: null as SessionTab | null,
  targetSftpItem: null as SftpFileItem | null,
  targetMemoGroupIdx: null as number | null,
  targetMemoItemIdx: null as number | null,
  targetMemoItem: null as MemoItem | null,
});

function openContextMenu(
  e: MouseEvent,
  type: "host" | "group" | "tab" | "terminal" | "sftp_item" | "sftp_bg" | "memo_item" | "memo_group" | "memo_bg",
  host?: HostItem,
  group?: DeviceGroup,
  sftpItem?: SftpFileItem,
  memoGroupIdx?: number,
  memoItemIdx?: number,
  memoItem?: MemoItem,
  tab?: SessionTab
) {
  e.preventDefault();
  e.stopPropagation();
  // 菜单实际高度通常仅为 120px~150px 左右，只有在紧贴底部不足 150px 时才做贴边限制
  const menuEstimatedHeight = (type === "sftp_item" || type === "host" || type === "terminal") ? 250 : 170;
  const menuEstimatedWidth = 190;
  const targetX = e.clientX + menuEstimatedWidth > window.innerWidth ? Math.max(10, window.innerWidth - menuEstimatedWidth - 10) : e.clientX;
  const targetY = e.clientY + menuEstimatedHeight > window.innerHeight ? Math.max(10, window.innerHeight - menuEstimatedHeight - 12) : e.clientY;

  contextMenu.value = {
    show: true,
    x: targetX,
    y: targetY,
    targetType: type,
    targetHost: host || null,
    targetGroup: group || null,
    targetTab: tab || null,
    targetSftpItem: sftpItem || null,
    targetMemoGroupIdx: memoGroupIdx !== undefined ? memoGroupIdx : null,
    targetMemoItemIdx: memoItemIdx !== undefined ? memoItemIdx : null,
    targetMemoItem: memoItem || null,
  };

  nextTick(() => {
    if (!contextMenuRef.value) return;
    const rect = contextMenuRef.value.getBoundingClientRect();
    const winW = window.innerWidth;
    const winH = window.innerHeight;
    let newX = contextMenu.value.x;
    let newY = contextMenu.value.y;

    if (rect.bottom > winH - 8) {
      newY = Math.max(8, winH - rect.height - 12);
    }
    if (rect.right > winW - 8) {
      newX = Math.max(8, winW - rect.width - 10);
    }
    contextMenu.value.x = newX;
    contextMenu.value.y = newY;
  });
}

function closeContextMenu() {
  contextMenu.value.show = false;
}

function toggleTerminalReadOnly() {
  const tab = activeSession.value;
  if (tab) tab.read_only = !tab.read_only;
  closeContextMenu();
}

// --- 弹窗体系 ---

// 1. 主机添加/编辑模态框
const keyGenModalShow = ref(false);
const keyGenDefaultComment = computed(() => {
  const user = hostModal.value.form.user || "root";
  const host = hostModal.value.form.host || "host";
  return `${user}@${host}`;
});

function openKeyGenModal() {
  keyGenModalShow.value = true;
}

function onKeyGenerated(payload: { privatePath: string; passphrase: string }) {
  hostModal.value.form.key_path = payload.privatePath;
  if (payload.passphrase) hostModal.value.form.passphrase = payload.passphrase;
}

const hostModal = ref({
  show: false,
  isEdit: false,
  mode: 'all' as 'all' | 'connection' | 'asset',
  form: {
    name: "",
    protocol: 'ssh' as 'ssh' | 'telnet' | 'serial' | 'local',
    local_shell: 'powershell' as 'powershell' | 'cmd',
    host: "",
    port: 22,
    user: "root",
    pwd: "",
    key_path: "",
    passphrase: "",
    serial_data_bits: 8,
    serial_parity: 'none' as 'none' | 'odd' | 'even',
    serial_stop_bits: 1,
    serial_flow_control: 'none' as 'none' | 'software' | 'hardware',
    serial_newline: 'cr' as 'cr' | 'lf' | 'crlf',
    serial_local_echo: false,
    serial_dtr: true,
    serial_rts: true,
    term_type: "auto",
    use_agent: false,
    memo: "",
    expire_date: "无限",
    expire_infinite: true,
    price: "0.00",
    currency: "CNY",
    traffic_total: "1000",
    traffic_total_unit: "GB",
    traffic_total_infinite: true,
    traffic_used: "0",
    traffic_used_unit: "GB",
    reset_day: 1,
    reset_hour: 0,
    reset_min: 0,
    reset_sec: 0,
    traffic_direction: "both",
    target_interface: "",
    traffic_price: "0.00",
    traffic_currency: "CNY",
  },
  originalHost: null as HostItem | null,
  targetGroup: null as DeviceGroup | null,
});

// 2. SSH 连接密码弹窗（主机未保存密码时）
const sshPasswordModal = ref({
  show: false,
  hostName: "",
  hostLabel: "",
  password: "",
  remember: false,
  error: "",
});
const sshPasswordInput = ref<HTMLInputElement | null>(null);
let sshPasswordResolver: ((value: { password: string; remember: boolean } | null) => void) | null = null;

function askSshPassword(tab: SessionTab): Promise<{ password: string; remember: boolean } | null> {
  if (sshPasswordResolver) {
    // A prompt for another connection is already open.
    return Promise.resolve(null);
  }
  sshPasswordModal.value = {
    show: true,
    hostName: tab.name,
    hostLabel: tab.host.user ? `${tab.host.user}@${tab.host.host}` : tab.host.host,
    password: "",
    remember: false,
    error: "",
  };
  nextTick(() => {
    sshPasswordInput.value?.focus();
  });
  return new Promise((resolve) => {
    sshPasswordResolver = resolve;
  });
}

function submitSshPassword() {
  const password = sshPasswordModal.value.password;
  if (!password) {
    sshPasswordModal.value.error = "请输入密码";
    sshPasswordInput.value?.focus();
    return;
  }
  const resolver = sshPasswordResolver;
  sshPasswordResolver = null;
  const result = { password, remember: sshPasswordModal.value.remember };
  sshPasswordModal.value.show = false;
  resolver?.(result);
}

function cancelSshPassword() {
  const resolver = sshPasswordResolver;
  sshPasswordResolver = null;
  sshPasswordModal.value.show = false;
  resolver?.(null);
}

// 2. 流量统计与校准模态框 (原版 TrafficStatsDialog)
const trafficStatsModal = ref({
  show: false,
  host: null as HostItem | null,
  source: "auto",
  customCommand: "",
  calibUsed: "",
  calibUnit: "GB",
  resetDay: 1,
  isSyncing: false,
  syncMessage: "",
  syncError: false,
  manualOverride: false,
  calibEditing: false,
});

// 3. SFTP 属性模态框 (原版 SFTPPropertiesDialog)
const sftpPropsModal = ref({
  show: false,
  name: "",
  full_path: "",
  is_dir: false,
  size_bytes: 0,
  mode_str: "",
  mode_oct: "",
  mtime_str: "",
});

// 4. SFTP 权限修改 (chmod) 模态框 (原版 SFTPChmodDialog)
const sftpChmodModal = ref({
  show: false,
  filePath: "",
  fileName: "",
  // 9个复选框矩阵
  u_r: true, u_w: true, u_x: true,
  g_r: true, g_w: false, g_x: true,
  o_r: true, o_w: false, o_x: true,
});

// 5. 新建文件夹 / 重命名 简单输入弹窗
const promptModal = ref({
  show: false,
  title: "",
  label: "",
  value: "",
  action: "" as "new_folder" | "rename_file" | "new_device_group" | "rename_device_group" | "new_memo_group" | "rename_memo_group",
  payload: null as any,
});

// 通用 Material 确认弹窗
interface ConfirmModalState {
  show: boolean;
  title: string;
  message: string;
  items?: string[];
  danger: boolean;
  confirmText: string;
  onConfirm: () => void;
}
const customConfirmModal = ref<ConfirmModalState>({
  show: false,
  title: "操作确认",
  message: "",
  items: [],
  danger: true,
  confirmText: "确认移除",
  onConfirm: () => {},
});

type TransferConflictChoice = "overwrite" | "rename" | "skip" | "cancel";
const transferConflictModal = ref<{
  show: boolean;
  title: string;
  message: string;
  fileName: string;
  resolve: ((choice: TransferConflictChoice) => void) | null;
}>({
  show: false,
  title: "文件已存在",
  message: "",
  fileName: "",
  resolve: null,
});

function askTransferConflict(title: string, message: string, fileName: string): Promise<TransferConflictChoice> {
  return new Promise((resolve) => {
    // 并发冲突（拖拽批量上传）时不能直接覆盖，否则上一个 Promise 永远悬挂。
    const pending = transferConflictModal.value.resolve;
    transferConflictModal.value = { show: true, title, message, fileName, resolve };
    if (pending) pending("cancel");
  });
}

function chooseTransferConflict(choice: TransferConflictChoice) {
  const resolve = transferConflictModal.value.resolve;
  transferConflictModal.value.show = false;
  transferConflictModal.value.resolve = null;
  if (resolve) resolve(choice);
}

// 6. 快捷指令编辑模态框
const memoEditModal = ref({
  show: false,
  isEdit: false,
  groupIndex: 0,
  itemIndex: -1,
  form: {
    name: "",
    cmd: "",
  }
});

// 7. 终端偏好与备份设置模态框 (1:1 严格对标 ssh8.py TerminalSettingsDialog)
const defaultGlobalFont = '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", sans-serif';
const appFontFamily = ref(defaultGlobalFont);
const terminalSettings = ref({
  font_family: "Consolas",
  font_size: 13,
  is_bold: false,
  cursor_style: "block" as "block" | "bar" | "underline",
  cursor_blink: true,
  blink_interval: 530,
  term_bg_mode: "dark" as "dark" | "light",
  terminal_fg_color: "#81C995",
  terminal_bg_color: "#0A0B0E",
  custom_bg_image: "",
  bg_image_fit: "cover" as "cover" | "contain" | "stretch" | "tile",
  bg_image_dim: 0.55,
  bg_image_auto_fg: true,
  minimize_to_tray: true,
  accent_color: "",
});
const terminalBgImageData = ref("");
const terminalBgImageLuminance = ref(0);
const showTerminalSettingsModal = ref(false);
const autostartEnabled = ref(false);

async function updateAutostart(enabled: boolean) {
  const previous = autostartEnabled.value;
  autostartEnabled.value = enabled;
  try {
    await invoke("set_autostart_enabled", { enabled });
  } catch (error) {
    autostartEnabled.value = previous;
    console.warn("[autostart] 修改开机自启失败:", error);
  }
}

function handleAutostartChange(event: Event) {
  const checked = (event.target as HTMLInputElement).checked;
  void updateAutostart(checked);
}

// 全局界面字体选择模态框
const showFontSelectorModal = ref(false);
const includeSystemFonts = ref(false);
const systemFontList = ref<string[]>([]);
const isReadingSystemFonts = ref(false);
const presetUiFonts = [
  { name: "系统推荐默认", value: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", sans-serif' },
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

function getDisplayFontName(fontStr: string) {
  if (!fontStr || fontStr.includes("-apple-system") || fontStr === defaultGlobalFont) {
    return "系统推荐默认";
  }
  const match = presetUiFonts.find(p => p.value === fontStr);
  if (match) return match.name;
  return fontStr.split(',')[0].replace(/['"]/g, '').trim();
}

function applyAppFont(font: string) {
  appFontFamily.value = font || defaultGlobalFont;
  document.documentElement.style.setProperty("--app-font-family", appFontFamily.value);
}

// 常见备选系统字体检测表 (含各类中英文常用字族)
const candidateSystemFonts = [
  "Arial", "Calibri", "Cambria", "Century Gothic", "Comic Sans MS", "Consolas",
  "Constantia", "Corbel", "Courier New", "Franklin Gothic Medium", "Georgia",
  "Impact", "Lucida Console", "Lucida Sans", "Malgun Gothic", "Meiryo",
  "Microsoft JhengHei", "Microsoft YaHei", "Microsoft YaHei UI", "MingLiU",
  "MS Gothic", "MS Mincho", "MS PGothic", "MS Sans Serif", "MS Serif",
  "Palatino Linotype", "Segoe Print", "Segoe Script", "Segoe UI", "Segoe UI Emoji",
  "Segoe UI Historic", "Segoe UI Symbol", "Segoe UI Variable Display", "Segoe UI Variable Small", "Segoe UI Variable Text",
  "SimHei", "SimSun", "NSimSun", "FangSong", "KaiTi", "Sylfaen", "Tahoma",
  "Times New Roman", "Trebuchet MS", "Verdana", "Yu Gothic", "Yu Mincho",
  "PingFang SC", "Heiti SC", "Songti SC", "Kaiti SC", "Apple Color Emoji",
  "HarmonyOS Sans SC", "MiSans", "OPPO Sans", "Source Han Sans SC", "Source Han Serif SC"
];

async function handleToggleSystemFonts() {
  if (!includeSystemFonts.value) {
    return;
  }
  if (systemFontList.value.length > 0) {
    return;
  }
  isReadingSystemFonts.value = true;
  try {
    // 优先通过标准 Local Font Access API 读取真实系统字体列表
    if ("queryLocalFonts" in window && typeof (window as any).queryLocalFonts === "function") {
      const fonts = await (window as any).queryLocalFonts();
      const set = new Set<string>();
      for (const f of fonts) {
        if (f.family) set.add(f.family);
      }
      systemFontList.value = Array.from(set).sort((a, b) => a.localeCompare(b));
    } else {
      // 降级使用 document.fonts.check 智能探针检测系统已安装字体
      const available: string[] = [];
      for (const name of candidateSystemFonts) {
        if (document.fonts && document.fonts.check(`12px "${name}"`)) {
          available.push(name);
        }
      }
      systemFontList.value = available.length > 0 ? available : candidateSystemFonts.slice(0, 20);
    }
  } catch (err) {
    console.warn("读取系统字体失败，启用探针列表:", err);
    systemFontList.value = candidateSystemFonts.slice(0, 25);
  } finally {
    isReadingSystemFonts.value = false;
  }
}

// 8. 备份与云同步管理模态框 (原版 BackupManagementDialog)
const backupModal = ref({
  show: false,
  webdav_url: "",
  webdav_user: "",
  webdav_pwd: "",
  webdav_path: "/QShell/qshell_backup.zip",
  status_msg: "",
  is_busy: false,
});
const showHostPassword = ref(false);
const showWebdavPassword = ref(false);

// 9. 从设备列表导入主机选择模态框 (原版 HostImportSelectionDialog)
const importSelectModal = ref({
  show: false,
  candidates: [] as HostItem[],
  selectedIndexes: [] as number[],
});

// --- 主机管理操作 ---
// --- 连接协议与串口管理 ---
const protocolOptions = [
  { label: "SSH (默认网络终端)", value: "ssh" },
  { label: "Telnet (网络终端)", value: "telnet" },
  { label: "Serial (串口硬件终端)", value: "serial" },
  { label: "Local (本地 PowerShell / CMD)", value: "local" },
];

// 终端类型：auto 时 SSH 会探测服务器上可用的 terminfo，Telnet 回退 xterm-256color。
const termTypeOptions = [
  { label: "自动（推荐）", value: "auto" },
  { label: "xterm-256color", value: "xterm-256color" },
  { label: "xterm", value: "xterm" },
  { label: "vt220", value: "vt220" },
  { label: "vt100", value: "vt100" },
  { label: "linux", value: "linux" },
  { label: "ansi", value: "ansi" },
];

const serialBaudRateOptions = [
  { label: "9600 (标准)", value: 9600 },
  { label: "19200", value: 19200 },
  { label: "38400", value: 38400 },
  { label: "57600", value: 57600 },
  { label: "115200 (高速常用)", value: 115200 },
  { label: "230400", value: 230400 },
  { label: "460800", value: 460800 },
  { label: "921600", value: 921600 },
];

const availableSerialPorts = ref<string[]>([]);
const isScanningPorts = ref(false);

const serialPortOptions = computed(() => {
  const scanned = [...availableSerialPorts.value];
  // 补充常用默认 COM 口，确保即便未插设备也能随时点击下拉选择
  const standard = ["COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8"];
  const set = new Set([...scanned, ...standard]);
  // 如果当前用户填写的 host 不在列表中，也动态补上
  if (hostModal.value.form.host && hostModal.value.form.host.trim()) {
    set.add(hostModal.value.form.host.trim());
  }
  return Array.from(set).map(p => ({
    label: scanned.includes(p) ? `${p} (已连接设备)` : p,
    value: p
  }));
});

async function refreshSerialPorts() {
  isScanningPorts.value = true;
  if (isTauri) {
    try {
      const ports = await invoke<string[]>("list_serial_ports");
      availableSerialPorts.value = ports || [];
      if (ports && ports.length > 0 && (!hostModal.value.form.host || hostModal.value.form.host === "COM1")) {
        hostModal.value.form.host = ports[0];
      }
    } catch (e) {
      console.warn("获取本机串口列表失败:", e);
    }
  }
  setTimeout(() => { isScanningPorts.value = false; }, 350);
}

const showBaudDropdown = ref(false);
const comboBaudRef = ref<HTMLElement | null>(null);

function selectBaudRate(rate: number) {
  hostModal.value.form.port = rate;
  showBaudDropdown.value = false;
}

function toggleBaudDropdown() {
  window.dispatchEvent(new CustomEvent("qshell-close-selects"));
  showBaudDropdown.value = !showBaudDropdown.value;
}

function handleCloseSelects() {
  showBaudDropdown.value = false;
}

function handleBaudOutsideClick(e: MouseEvent) {
  if (comboBaudRef.value && !comboBaudRef.value.contains(e.target as Node)) {
    showBaudDropdown.value = false;
  }
}

function onProtocolChange(newProto: "ssh" | "telnet" | "serial" | "local") {
  hostModal.value.form.protocol = newProto;
  if (newProto === "ssh") {
    hostModal.value.form.port = 22;
    if (hostModal.value.form.host.toUpperCase().startsWith("COM")) {
      hostModal.value.form.host = "127.0.0.1";
    }
  } else if (newProto === "telnet") {
    hostModal.value.form.port = 23;
    if (hostModal.value.form.host.toUpperCase().startsWith("COM")) {
      hostModal.value.form.host = "127.0.0.1";
    }
  } else if (newProto === "serial") {
    hostModal.value.form.port = 9600;
    refreshSerialPorts();
    if (!hostModal.value.form.host || !hostModal.value.form.host.toUpperCase().startsWith("COM")) {
      hostModal.value.form.host = availableSerialPorts.value[0] || "COM1";
    }
  }
}
function openAddHostModal(modeOrEvent?: 'all' | 'connection' | Event, targetGroup?: DeviceGroup | null) {
  showHostPassword.value = false;
  const mode = typeof modeOrEvent === 'string' ? modeOrEvent : 'connection';
  const group = targetGroup || (contextMenu.value.targetGroup || null);
  hostModal.value = {
    show: true,
    isEdit: false,
    mode: mode,
    form: {
      name: "新服务器",
      protocol: "ssh" as "ssh" | "telnet" | "serial" | "local",
      local_shell: "powershell" as "powershell" | "cmd",
      host: "127.0.0.1",
      port: 22,
      user: "root",
      pwd: "",
      key_path: "",
      passphrase: "",
      serial_data_bits: 8,
      serial_parity: 'none' as 'none' | 'odd' | 'even',
      serial_stop_bits: 1,
      serial_flow_control: 'none' as 'none' | 'software' | 'hardware',
      serial_newline: 'cr' as 'cr' | 'lf' | 'crlf',
      serial_local_echo: false,
      serial_dtr: true,
      serial_rts: true,
      term_type: "auto",
      use_agent: false,
      memo: "",
      expire_date: "",
      expire_infinite: true,
      price: "0.00",
      currency: "CNY",
      traffic_total: "1000",
      traffic_total_unit: "GB",
      traffic_total_infinite: true,
      traffic_used: "0",
      traffic_used_unit: "GB",
      reset_day: 1,
      reset_hour: 0,
      reset_min: 0,
    reset_sec: 0,
      traffic_direction: "both",
      target_interface: "",
      traffic_price: "0.00",
      traffic_currency: "CNY",
    },
    originalHost: null,
    targetGroup: group,
  };
  closeContextMenu();
}

function openEditHostModal(h: HostItem, mode: 'all' | 'connection' | 'asset' = 'all') {
  showHostPassword.value = false;
  const exp = h.details?.expire_date || "无限";
  const isExpInf = exp === "无限" || !exp;
  hostModal.value = {
    show: true,
    isEdit: true,
    mode: mode,
    form: {
      protocol: (h.protocol || "ssh") as "ssh" | "telnet" | "serial" | "local",
      local_shell: (h.local_shell || "powershell") as "powershell" | "cmd",
      name: h.name,
      host: h.host,
      port: h.port,
      user: h.user,
      pwd: h.pwd || "",
      key_path: h.key_path || "",
      passphrase: h.passphrase || "",
      serial_data_bits: h.serial_data_bits ?? 8,
      serial_parity: (h.serial_parity || "none") as "none" | "odd" | "even",
      serial_stop_bits: h.serial_stop_bits ?? 1,
      serial_flow_control: (h.serial_flow_control || "none") as "none" | "software" | "hardware",
      serial_newline: (h.serial_newline || "cr") as "cr" | "lf" | "crlf",
      serial_local_echo: h.serial_local_echo ?? false,
      serial_dtr: h.serial_dtr ?? true,
      serial_rts: h.serial_rts ?? true,
      term_type: h.term_type || "auto",
      use_agent: !!h.use_agent,
      memo: h.details?.memo || "",
      expire_date: isExpInf ? "" : exp,
      expire_infinite: isExpInf,
      price: h.details?.price || "0.00",
      currency: h.details?.currency || "CNY",
      traffic_total: h.details?.traffic_total || "1000",
      traffic_total_unit: h.details?.traffic_total_unit || "GB",
      traffic_total_infinite: h.details?.traffic_total_infinite ?? true,
      traffic_used: h.details?.traffic_used || "0",
      traffic_used_unit: h.details?.traffic_used_unit || "GB",
      reset_day: h.details?.reset_day || 1,
      reset_hour: h.details?.reset_hour ?? 0,
      reset_min: h.details?.reset_min ?? 0,
      reset_sec: h.details?.reset_sec ?? 0,
      traffic_direction: h.details?.traffic_direction || "both",
      target_interface: h.details?.target_interface || "",
      traffic_price: h.details?.traffic_price || "0.00",
      traffic_currency: h.details?.traffic_currency || "CNY",
    },
    originalHost: h,
    targetGroup: null,
  };
  closeContextMenu();
}

async function browseKeyFile() {
  const selected = await openLocalFileBrowser({
    action: "open",
    title: "选择 SSH 私钥文件 (.pem, id_rsa)",
  });
  if (selected) {
    hostModal.value.form.key_path = selected;
  }
}

async function saveHostForm() {
  const f = hostModal.value.form;
  if (!f.name || !f.host) return;

  const finalExpireDate = f.expire_infinite ? "无限" : (f.expire_date || "无限");
  const prevUsed = hostModal.value.originalHost?.details?.traffic_used;
  const prevUsedUnit = hostModal.value.originalHost?.details?.traffic_used_unit;
  const prevReset: ResetSchedule | null = hostModal.value.originalHost?.details
    ? {
        day: hostModal.value.originalHost.details.reset_day ?? 1,
        hour: hostModal.value.originalHost.details.reset_hour ?? 0,
        min: hostModal.value.originalHost.details.reset_min ?? 0,
        sec: hostModal.value.originalHost.details.reset_sec ?? 0,
      }
    : null;

  if (hostModal.value.isEdit && hostModal.value.originalHost) {
    const orig = hostModal.value.originalHost;
    if (hostModal.value.mode === 'asset') {
      // 仅编辑资产与流量配额 + 备注说明
      orig.details = {
        ...orig.details,
        memo: f.memo,
        expire_date: finalExpireDate,
        price: f.price,
        currency: f.currency,
        traffic_total: f.traffic_total,
        traffic_total_unit: f.traffic_total_unit,
        traffic_total_infinite: f.traffic_total_infinite,
        traffic_used: f.traffic_used,
        traffic_used_unit: f.traffic_used_unit,
        reset_day: Number(f.reset_day) || 1,
        reset_hour: Number(f.reset_hour) || 0,
        reset_min: Number(f.reset_min) || 0,
        reset_sec: Number(f.reset_sec) || 0,
        traffic_direction: f.traffic_direction,
        target_interface: f.target_interface || "",
        traffic_price: f.traffic_price,
        traffic_currency: f.traffic_currency,
      };
    } else if (hostModal.value.mode === 'connection') {
      // 仅编辑连接与认证
      orig.name = f.name;
      orig.protocol = f.protocol;
      orig.host = f.host;
      orig.port = Number(f.port) || 22;
      orig.user = f.user;
      orig.pwd = f.pwd || undefined;
      orig.key_path = f.key_path || undefined;
      orig.passphrase = f.passphrase || undefined;
      orig.serial_data_bits = f.protocol === 'serial' ? f.serial_data_bits : undefined;
      orig.serial_parity = f.protocol === 'serial' ? f.serial_parity : undefined;
      orig.serial_stop_bits = f.protocol === 'serial' ? f.serial_stop_bits : undefined;
      orig.serial_flow_control = f.protocol === 'serial' ? f.serial_flow_control : undefined;
      orig.serial_newline = f.protocol === 'serial' ? f.serial_newline : undefined;
      orig.serial_local_echo = f.protocol === 'serial' ? f.serial_local_echo : undefined;
      orig.serial_dtr = f.protocol === 'serial' ? f.serial_dtr : undefined;
      orig.serial_rts = f.protocol === 'serial' ? f.serial_rts : undefined;
    } else {
      // 全部
      orig.name = f.name;
      orig.protocol = f.protocol;
      orig.host = f.host;
      orig.port = Number(f.port) || 22;
      orig.user = f.user;
      orig.pwd = f.pwd || undefined;
      orig.key_path = f.key_path || undefined;
      orig.passphrase = f.passphrase || undefined;
      orig.serial_data_bits = f.protocol === 'serial' ? f.serial_data_bits : undefined;
      orig.serial_parity = f.protocol === 'serial' ? f.serial_parity : undefined;
      orig.serial_stop_bits = f.protocol === 'serial' ? f.serial_stop_bits : undefined;
      orig.serial_flow_control = f.protocol === 'serial' ? f.serial_flow_control : undefined;
      orig.serial_newline = f.protocol === 'serial' ? f.serial_newline : undefined;
      orig.serial_local_echo = f.protocol === 'serial' ? f.serial_local_echo : undefined;
      orig.serial_dtr = f.protocol === 'serial' ? f.serial_dtr : undefined;
      orig.serial_rts = f.protocol === 'serial' ? f.serial_rts : undefined;
      orig.details = {
        ...orig.details,
        memo: f.memo,
        expire_date: finalExpireDate,
        price: f.price,
        currency: f.currency,
        traffic_total: f.traffic_total,
        traffic_total_unit: f.traffic_total_unit,
        traffic_total_infinite: f.traffic_total_infinite,
        traffic_used: f.traffic_used,
        traffic_used_unit: f.traffic_used_unit,
        reset_day: Number(f.reset_day) || 1,
        reset_hour: Number(f.reset_hour) || 0,
        reset_min: Number(f.reset_min) || 0,
        reset_sec: Number(f.reset_sec) || 0,
        traffic_direction: f.traffic_direction,
        target_interface: f.target_interface || "",
        traffic_price: f.traffic_price,
        traffic_currency: f.traffic_currency,
      };
    }

    // 在资产表单里手工改过“已用流量”时，同步周期字节并视为手动校准
    if (hostModal.value.mode !== 'connection' && orig.details) {
      const usedChanged =
        String(prevUsed ?? "") !== String(f.traffic_used ?? "") ||
        String(prevUsedUnit ?? "") !== String(f.traffic_used_unit ?? "");
      if (usedChanged) {
        const usedValue = Number(f.traffic_used) || 0;
        orig.details.traffic_cycle_used_bytes = Math.max(
          0,
          Math.round(usedValue * trafficUnitMultiplier(f.traffic_used_unit))
        );
        orig.details.traffic_delta_bytes = 0;
        orig.details.last_raw_bytes = 0;
        orig.details.traffic_counter_sig = "";
        orig.details.traffic_boot_id = "";
        orig.details.traffic_iface_baseline = "";
        orig.details.traffic_manual_override = true;
        orig.details.traffic_vnstat_anchor_bytes = undefined;
        orig.details.traffic_vnstat_anchor_ts = undefined;
        orig.details.traffic_cycle_anchor_used_bytes = undefined;
      }
    }

    if (hostModal.value.mode !== 'connection' && orig.details && prevReset) {
      markResetScheduleChanged(orig.details, prevReset, {
        day: Number(f.reset_day) || 1,
        hour: Number(f.reset_hour) || 0,
        min: Number(f.reset_min) || 0,
        sec: Number(f.reset_sec) || 0,
      });
    }

    // 跨列表同步（确保设备列表与主机管理列表一致）
    const syncHostInList = (list: HostItem[]) => {
      const found = list.find(x => x !== orig && (x.host === orig.host || x.name === orig.name));
      if (found) {
        if (hostModal.value.mode === 'asset') {
          found.details = orig.details;
        } else if (hostModal.value.mode === 'connection') {
          found.name = orig.name;
          found.protocol = orig.protocol;
          found.host = orig.host;
          found.port = orig.port;
          found.user = orig.user;
          found.pwd = orig.pwd;
          found.key_path = orig.key_path;
          found.passphrase = orig.passphrase;
          found.serial_data_bits = orig.serial_data_bits;
          found.serial_parity = orig.serial_parity;
          found.serial_stop_bits = orig.serial_stop_bits;
          found.serial_flow_control = orig.serial_flow_control;
          found.serial_newline = orig.serial_newline;
          found.serial_local_echo = orig.serial_local_echo;
          found.serial_dtr = orig.serial_dtr;
          found.serial_rts = orig.serial_rts;
          found.local_shell = orig.local_shell;
        } else {
          found.name = orig.name;
          found.protocol = orig.protocol;
          found.host = orig.host;
          found.port = orig.port;
          found.user = orig.user;
          found.pwd = orig.pwd;
          found.key_path = orig.key_path;
          found.passphrase = orig.passphrase;
          found.serial_data_bits = orig.serial_data_bits;
          found.serial_parity = orig.serial_parity;
          found.serial_stop_bits = orig.serial_stop_bits;
          found.serial_flow_control = orig.serial_flow_control;
          found.serial_newline = orig.serial_newline;
          found.serial_local_echo = orig.serial_local_echo;
          found.serial_dtr = orig.serial_dtr;
          found.serial_rts = orig.serial_rts;
          found.local_shell = orig.local_shell;
          found.term_type = orig.term_type;
          found.use_agent = orig.use_agent;
          found.details = orig.details;
        }
      }
    };
    configData.value.managed_hosts && syncHostInList(configData.value.managed_hosts);
    configData.value.devices.forEach(g => syncHostInList(g.hosts));
  } else {
    const targetHost: HostItem = {
      name: f.name,
      protocol: f.protocol,
      host: f.host,
      port: Number(f.port) || 22,
      user: f.user,
      pwd: f.pwd || undefined,
      key_path: f.key_path || undefined,
      passphrase: f.passphrase || undefined,
      serial_data_bits: f.protocol === 'serial' ? f.serial_data_bits : undefined,
      serial_parity: f.protocol === 'serial' ? f.serial_parity : undefined,
      serial_stop_bits: f.protocol === 'serial' ? f.serial_stop_bits : undefined,
      serial_flow_control: f.protocol === 'serial' ? f.serial_flow_control : undefined,
      serial_newline: f.protocol === 'serial' ? f.serial_newline : undefined,
      serial_local_echo: f.protocol === 'serial' ? f.serial_local_echo : undefined,
      serial_dtr: f.protocol === 'serial' ? f.serial_dtr : undefined,
      serial_rts: f.protocol === 'serial' ? f.serial_rts : undefined,
      local_shell: f.protocol === 'local' ? f.local_shell : undefined,
      term_type: f.protocol === 'serial' || f.protocol === 'local' ? undefined : (f.term_type || "auto"),
      use_agent: f.protocol === 'ssh' ? !!f.use_agent : undefined,
      details: {
        memo: f.memo,
        expire_date: finalExpireDate,
        price: f.price,
        currency: f.currency,
        traffic_total: f.traffic_total,
        traffic_total_unit: f.traffic_total_unit,
        traffic_total_infinite: f.traffic_total_infinite,
        traffic_used: f.traffic_used,
        traffic_used_unit: f.traffic_used_unit,
        reset_day: Number(f.reset_day) || 1,
        reset_hour: Number(f.reset_hour) || 0,
        reset_min: Number(f.reset_min) || 0,
        reset_sec: Number(f.reset_sec) || 0,
        traffic_direction: f.traffic_direction,
        target_interface: f.target_interface || "",
        traffic_price: f.traffic_price,
        traffic_currency: f.traffic_currency,
      },
    };
    let destGroup = hostModal.value.targetGroup
      ? configData.value.devices.find(g => g === hostModal.value.targetGroup || g.group === hostModal.value.targetGroup?.group)
      : null;

    // 从设备列表空白处新增时不再默认塞进第一个分组（VPS）。
    // 没有明确目标分组时统一放入独立的“未分组”。
    if (!destGroup) {
      destGroup = configData.value.devices.find(g => g.group === "未分组");
      if (!destGroup) {
        destGroup = { group: "未分组", hosts: [] };
        configData.value.devices.push(destGroup);
      }
    }
    destGroup.hosts.push(targetHost);

    // 从设备列表新增（connection 模式）只进入设备列表，不自动同步到“主机管理”。
    // 主机管理页的新增（all/asset 模式）才同步到 managed_hosts。
    if (hostModal.value.mode !== 'connection') {
      if (!configData.value.managed_hosts) {
        configData.value.managed_hosts = [];
      }
      if (!configData.value.managed_hosts.some(x => x.host === targetHost.host && x.port === targetHost.port)) {
        configData.value.managed_hosts.push(targetHost);
      }
    }
  }

  evaluateHostAlerts();
  await invoke("save_hosts_config", { config: configData.value });
  hostModal.value.show = false;
}

// 从设备列表中删除主机 (左侧设备树右键)
async function deleteHost(h: HostItem) {
  closeContextMenu();
  configData.value.devices.forEach(g => {
    g.hosts = g.hosts.filter(item => item !== h && !(item.host === h.host && item.port === h.port));
  });
  await invoke("save_hosts_config", { config: configData.value });
}

// 从主机管理列表中删除主机 (仅移除卡片，不影响设备列表)
async function deleteManagedHost(h: HostItem) {
  configData.value.managed_hosts = configData.value.managed_hosts.filter(
    item => item !== h && !(item.host === h.host && item.port === h.port)
  );
  evaluateHostAlerts();
  await invoke("save_hosts_config", { config: configData.value });
}

function confirmDeleteHost(h: HostItem) {
  customConfirmModal.value = {
    show: true,
    title: "移除主机卡片",
    message: `确定要从主机管理中移除 "${h.name}" 吗？此操作不会删除设备列表中的主机。`,
    danger: true,
    confirmText: "确定移除",
    onConfirm: () => {
      deleteManagedHost(h);
      customConfirmModal.value.show = false;
    },
  };
}

// 流量统计弹窗 (TrafficStatsDialog)
function trafficUnitMultiplier(unit?: string): number {
  const u = String(unit || "GB").toUpperCase();
  if (u === "B" || u === "BYTES") return 1;
  if (u.includes("KB")) return 1024;
  if (u.includes("MB")) return 1024 ** 2;
  if (u.includes("TB")) return 1024 ** 4;
  return 1024 ** 3;
}

interface ResetSchedule {
  day: number;
  hour: number;
  min: number;
  sec: number;
}

function resetOccurrenceThisMonth(schedule: ResetSchedule, now = new Date()): Date {
  const year = now.getFullYear();
  const month = now.getMonth();
  const maxDay = new Date(year, month + 1, 0).getDate();
  const day = Math.min(Math.max(Number(schedule.day) || 1, 1), maxDay);
  return new Date(
    year,
    month,
    day,
    Math.min(Math.max(Number(schedule.hour) || 0, 0), 23),
    Math.min(Math.max(Number(schedule.min) || 0, 0), 59),
    Math.min(Math.max(Number(schedule.sec) || 0, 0), 59),
  );
}

// 用户修改重置时间后：不立即回算 vnstat，统一从下一次重置开始重新锚定。
// 如果新的重置时刻已经过去，则本周期强制走网卡增量，避免不完整的 vnstat 绝对值。
function markResetScheduleChanged(d: HostDetails, oldSchedule: ResetSchedule, newSchedule: ResetSchedule) {
  if (
    oldSchedule.day === newSchedule.day &&
    oldSchedule.hour === newSchedule.hour &&
    oldSchedule.min === newSchedule.min &&
    oldSchedule.sec === newSchedule.sec
  ) {
    return;
  }
  const now = new Date();
  const occurred = resetOccurrenceThisMonth(newSchedule, now).getTime() <= now.getTime();
  d.last_reset_timestamp = now.getTime() / 1000;
  d.traffic_force_nic = occurred;
  d.traffic_vnstat_anchor_bytes = undefined;
  d.traffic_vnstat_anchor_ts = undefined;
  d.traffic_cycle_anchor_used_bytes = undefined;
}

function trafficSourceLabel(source?: string): string {
  switch (source) {
    case "auto": return "自动 (vnstat → 本地计数器)";
    case "vnstat": return "vnstat";
    case "proc": return "本地计数器 (/proc/net/dev)";
    case "custom": return "自定义命令";
    case "none": return "未同步";
    default: return source || "尚未同步";
  }
}

function formatTrafficSyncTime(ts?: number): string {
  if (!ts || ts <= 0) return "-";
  const d = new Date(ts * 1000);
  if (isNaN(d.getTime())) return "-";
  return d.toLocaleString("zh-CN", { hour12: false });
}

function openTrafficStatsModal(h: HostItem) {
  closeContextMenu();
  const err = String(h.details?.traffic_source_error || "").trim();
  const effective = h.details?.traffic_source_effective || "";
  const lastTs = h.details?.traffic_last_sync_ts || 0;
  let message = "";
  if (lastTs > 0) {
    message = `上次同步: ${formatTrafficSyncTime(lastTs)} · 来源: ${trafficSourceLabel(effective)}`;
  }
  if (err) {
    message += (message ? " · " : "") + err;
  }
  trafficStatsModal.value = {
    show: true,
    host: h,
    source: h.details?.traffic_source || "auto",
    customCommand: h.details?.traffic_custom_command || "",
    calibUsed: h.details?.traffic_used || "0",
    calibUnit: h.details?.traffic_used_unit || "GB",
    resetDay: h.details?.reset_day || 1,
    isSyncing: false,
    syncMessage: message,
    syncError: !!(err && effective === "none"),
    manualOverride: !!h.details?.traffic_manual_override,
    calibEditing: false,
  };
}

async function saveTrafficStats() {
  const m = trafficStatsModal.value;
  if (!m.host) return;

  if (m.source === "custom" && !m.customCommand.trim()) {
    m.syncError = true;
    m.syncMessage = "选择自定义命令时，请填写远程执行命令。";
    return;
  }
  const calibValue = parseFloat(m.calibUsed);
  if (m.calibUsed.trim() !== "" && (isNaN(calibValue) || calibValue < 0)) {
    m.syncError = true;
    m.syncMessage = "请输入有效的已用流量数字。";
    return;
  }

  if (!m.host.details) m.host.details = {};
  const d = m.host.details;
  const oldReset: ResetSchedule = {
    day: d.reset_day ?? 1,
    hour: d.reset_hour ?? 0,
    min: d.reset_min ?? 0,
    sec: d.reset_sec ?? 0,
  };
  d.traffic_source = m.source;
  d.traffic_custom_command = m.customCommand;
  d.reset_day = m.resetDay;
  d.traffic_used = m.calibUsed.trim() || "0";
  d.traffic_used_unit = m.calibUnit;
  d.traffic_cycle_used_bytes = Math.max(0, Math.round((isNaN(calibValue) ? 0 : calibValue) * trafficUnitMultiplier(m.calibUnit)));
  d.traffic_delta_bytes = 0;
  d.last_raw_bytes = 0;
  d.traffic_counter_sig = "";
  d.traffic_boot_id = "";
  d.traffic_iface_baseline = "";
  d.traffic_source_error = "";
  d.traffic_manual_override = m.manualOverride;
  d.traffic_vnstat_anchor_bytes = undefined;
  d.traffic_vnstat_anchor_ts = undefined;
  d.traffic_cycle_anchor_used_bytes = undefined;
  markResetScheduleChanged(d, oldReset, {
    day: m.resetDay,
    hour: d.reset_hour ?? 0,
    min: d.reset_min ?? 0,
    sec: d.reset_sec ?? 0,
  });
  trafficAuditFailures.delete(trafficAuditKey(m.host));
  trafficAuditNextAt.set(trafficAuditKey(m.host), Date.now() + trafficAuditIntervalFor(m.host));

  await persistTrafficConfig();
  evaluateHostAlerts();
  m.show = false;
}

// =========================================================================
// 流量审计调度：定时、串行保存、并发限制、失败退避
// =========================================================================
const TRAFFIC_AUDIT_TICK_MS = 5_000;
const TRAFFIC_AUDIT_INTERVAL_MS = 5 * 60_000;
const TRAFFIC_AUDIT_ACTIVE_INTERVAL_MS = 60_000;
const TRAFFIC_AUDIT_MAX_CONCURRENCY = 3;
const TRAFFIC_AUDIT_STARTUP_JITTER_MS = 30_000;
const TRAFFIC_AUDIT_BACKOFF_BASE_MS = 30_000;
const TRAFFIC_AUDIT_BACKOFF_MAX_MS = 30 * 60_000;
const TRAFFIC_ANCHOR_MAX_CONCURRENCY = 8;
const TRAFFIC_ANCHOR_RETRY_MS = 5_000;
const TRAFFIC_ANCHOR_MAX_ATTEMPTS = 6;
const TRAFFIC_ANCHOR_WINDOW_MS = 120_000;

let trafficAuditTimer: number | null = null;
let trafficAuditSchedulerBusy = false;
let trafficAuditSchedulerStarted = false;
let trafficConfigSaveChain: Promise<void> = Promise.resolve();
const trafficAuditInFlight = new Map<string, Promise<TrafficAuditResult>>();
const trafficAuditNextAt = new Map<string, number>();
const trafficAuditFailures = new Map<string, number>();

interface TrafficAnchorState {
  resetTs: number;
  attempts: number;
  nextAt: number;
}
const trafficAnchorState = new Map<string, TrafficAnchorState>();
const trafficAnchorInFlight = new Set<string>();

function trafficAuditKey(h: HostItem): string {
  return [h.name, h.host, h.port, h.user].join("|");
}

function ensureHostDetails(h: HostItem): HostDetails {
  if (!h.details) h.details = {};
  return h.details;
}

function buildTrafficAuditParams(h: HostItem, source: string, customCommand: string) {
  const d = ensureHostDetails(h);
  return {
    host: h.host,
    port: h.port,
    username: h.user,
    password: h.pwd || null,
    private_key: h.key_path || null,
    passphrase: h.passphrase || null,
    source,
    custom_command: customCommand,
    reset_day: d.reset_day ?? 1,
    reset_hour: d.reset_hour ?? 0,
    reset_minute: d.reset_min ?? 0,
    reset_second: d.reset_sec ?? 0,
    direction: d.traffic_direction || "both",
    target_interface: d.target_interface || "",
    traffic_used: d.traffic_used || "0",
    traffic_used_unit: d.traffic_used_unit || "GB",
    traffic_cycle_used_bytes: d.traffic_cycle_used_bytes ?? null,
    traffic_delta_bytes: d.traffic_delta_bytes ?? 0,
    last_raw_bytes: d.last_raw_bytes ?? 0,
    traffic_counter_sig: d.traffic_counter_sig || "",
    traffic_boot_id: d.traffic_boot_id || "",
    last_reset_timestamp: d.last_reset_timestamp ?? 0,
    traffic_iface_baseline: d.traffic_iface_baseline || "",
    force_nic: !!d.traffic_force_nic,
  };
}

function applyTrafficAuditResult(h: HostItem, res: TrafficAuditResult) {
  const d = ensureHostDetails(h);
  const manual = !!d.traffic_manual_override;
  const previousUnit = d.traffic_used_unit || "GB";
  const previousCycle = d.traffic_cycle_used_bytes ?? 0;
  const previousLastReset = d.last_reset_timestamp ?? 0;
  const resetHappened =
    previousLastReset > 0 && res.last_reset_timestamp > previousLastReset + 1;
  const source = res.traffic_source_effective;

  d.traffic_delta_bytes = res.traffic_delta_bytes;
  d.last_raw_bytes = res.last_raw_bytes;
  d.traffic_counter_sig = res.traffic_counter_sig;
  d.traffic_boot_id = res.traffic_boot_id;
  d.traffic_iface_baseline = res.traffic_iface_baseline;
  d.last_reset_timestamp = res.last_reset_timestamp;
  d.traffic_source_effective = source;
  d.traffic_last_sync_ts = res.traffic_last_sync_ts;

  let cycle = res.traffic_cycle_used_bytes;

  if (resetHappened) {
    // 到达用户设置的结算时刻：服务商流量归零，重新建立本周期锚点。
    cycle = 0;
    d.traffic_manual_override = false;
    d.traffic_force_nic = false;
    if (source === "vnstat") {
      d.traffic_vnstat_anchor_bytes = res.traffic_cycle_used_bytes;
      d.traffic_vnstat_anchor_ts = res.traffic_last_sync_ts;
      d.traffic_cycle_anchor_used_bytes = 0;
    } else {
      d.traffic_vnstat_anchor_bytes = undefined;
      d.traffic_vnstat_anchor_ts = undefined;
      d.traffic_cycle_anchor_used_bytes = undefined;
    }
  } else if (source === "vnstat") {
    const anchor = d.traffic_vnstat_anchor_bytes;
    if (anchor != null) {
      if (res.traffic_cycle_used_bytes >= anchor) {
        const anchorUsed = d.traffic_cycle_anchor_used_bytes ?? previousCycle;
        cycle = anchorUsed + (res.traffic_cycle_used_bytes - anchor);
      } else {
        // vnstat 数据被重置或重装：遵循 vnstat 当前值并重新锚定
        cycle = res.traffic_cycle_used_bytes;
        d.traffic_vnstat_anchor_bytes = res.traffic_cycle_used_bytes;
        d.traffic_vnstat_anchor_ts = res.traffic_last_sync_ts;
        d.traffic_cycle_anchor_used_bytes = cycle;
      }
    } else {
      // 本周期第一次看到完整 vnstat：
      // 无手工基准时直接采用绝对周期值；有手工基准时保留基准，后续只累加增量。
      cycle = manual ? previousCycle : res.traffic_cycle_used_bytes;
      d.traffic_vnstat_anchor_bytes = res.traffic_cycle_used_bytes;
      d.traffic_vnstat_anchor_ts = res.traffic_last_sync_ts;
      d.traffic_cycle_anchor_used_bytes = cycle;
    }
  } else {
    // 网卡/自定义/无数据：后端已在传入周期值上累加增量
    cycle = res.traffic_cycle_used_bytes;
    d.traffic_vnstat_anchor_bytes = undefined;
    d.traffic_vnstat_anchor_ts = undefined;
    d.traffic_cycle_anchor_used_bytes = undefined;
  }

  d.traffic_cycle_used_bytes = cycle;
  d.traffic_used_unit = previousUnit;
  d.traffic_used = (cycle / trafficUnitMultiplier(previousUnit)).toFixed(2);

  if (res.traffic_source_error) {
    d.traffic_source_error = res.traffic_source_error;
  } else if (manual) {
    d.traffic_source_error = "手动基准已保留，自动增量已累加";
  } else {
    d.traffic_source_error = "";
  }

  refreshOpenTrafficStatsModal(h);
}

// 定时审计更新了 host.details 时，同步刷新已经打开的流量弹窗
function refreshOpenTrafficStatsModal(h: HostItem) {
  const m = trafficStatsModal.value;
  if (!m.show || m.host !== h) return;
  const d = h.details || {};
  const err = String(d.traffic_source_error || "").trim();
  let message = "";
  if (d.traffic_last_sync_ts) {
    message = `上次同步: ${formatTrafficSyncTime(d.traffic_last_sync_ts)} · 来源: ${trafficSourceLabel(d.traffic_source_effective)}`;
  }
  if (err) {
    message += (message ? " · " : "") + err;
  }
  m.syncMessage = message;
  m.syncError = d.traffic_source_effective === "none";
  if (!m.calibEditing) {
    const cycle = d.traffic_cycle_used_bytes ?? 0;
    m.calibUsed = (cycle / trafficUnitMultiplier(m.calibUnit)).toFixed(2);
  }
}

// 同一个主机在同一时刻只允许一个审计任务，避免手动点击和定时任务并发
function runHostTrafficAudit(h: HostItem): Promise<TrafficAuditResult> {
  const key = trafficAuditKey(h);
  const existing = trafficAuditInFlight.get(key);
  if (existing) return existing;

  const d = ensureHostDetails(h);
  const source = d.traffic_source || "auto";
  const customCommand = d.traffic_custom_command || "";
  const tracked = invoke<TrafficAuditResult>("audit_host_traffic", {
    params: buildTrafficAuditParams(h, source, customCommand),
  }).finally(() => {
    trafficAuditInFlight.delete(key);
  });
  trafficAuditInFlight.set(key, tracked);
  return tracked;
}

// 串行写配置，避免 100 台主机同时回写造成文件竞争
function persistTrafficConfig(): Promise<void> {
  if (!isTauri) return Promise.resolve();
  trafficConfigSaveChain = trafficConfigSaveChain
    .catch(() => {})
    .then(async () => {
      await invoke("save_hosts_config", { config: configData.value });
    })
    .catch((err) => {
      console.warn("[traffic-audit] 保存配置失败:", err);
    });
  return trafficConfigSaveChain;
}

function trafficAuditIntervalFor(h: HostItem): number {
  const active = activeSession.value?.host;
  if (
    active &&
    active.host === h.host &&
    active.port === h.port &&
    (active.user || "") === (h.user || "")
  ) {
    return TRAFFIC_AUDIT_ACTIVE_INTERVAL_MS;
  }
  return TRAFFIC_AUDIT_INTERVAL_MS;
}

function trafficAuditBackoff(failures: number): number {
  const capped = Math.min(Math.max(failures, 1), 8);
  return Math.min(
    TRAFFIC_AUDIT_BACKOFF_MAX_MS,
    TRAFFIC_AUDIT_BACKOFF_BASE_MS * Math.pow(2, capped - 1)
  );
}

function markTrafficAuditOutcome(h: HostItem, res: TrafficAuditResult) {
  const key = trafficAuditKey(h);
  if (res.traffic_source_effective === "none") {
    const failures = (trafficAuditFailures.get(key) || 0) + 1;
    trafficAuditFailures.set(key, failures);
    trafficAuditNextAt.set(key, Date.now() + trafficAuditBackoff(failures));
  } else {
    trafficAuditFailures.delete(key);
    trafficAuditNextAt.set(key, Date.now() + trafficAuditIntervalFor(h));
  }
}

function dispatchScheduledTrafficAudit(h: HostItem) {
  const key = trafficAuditKey(h);
  if (trafficAuditInFlight.has(key)) return;

  trafficAuditNextAt.set(key, Date.now() + trafficAuditIntervalFor(h));
  runHostTrafficAudit(h)
    .then((res) => {
      applyTrafficAuditResult(h, res);
      markTrafficAuditOutcome(h, res);
      evaluateHostAlerts();
      void persistTrafficConfig();
    })
    .catch((err) => {
      const failures = (trafficAuditFailures.get(key) || 0) + 1;
      trafficAuditFailures.set(key, failures);
      trafficAuditNextAt.set(key, Date.now() + trafficAuditBackoff(failures));
      console.warn(`[traffic-audit] ${h.name} 审计失败:`, err);
    });
}

function anchorWindowFor(h: HostItem, now: number): { resetTs: number; state: TrafficAnchorState } | null {
  const d = h.details;
  if (!d) return null;
  const occurred = resetOccurrenceThisMonth(
    {
      day: d.reset_day ?? 1,
      hour: d.reset_hour ?? 0,
      min: d.reset_min ?? 0,
      sec: d.reset_sec ?? 0,
    },
    new Date(now),
  );
  const resetTs = occurred.getTime() / 1000;
  if (now < occurred.getTime() || now > occurred.getTime() + TRAFFIC_ANCHOR_WINDOW_MS) {
    return null;
  }
  const lastReset = d.last_reset_timestamp ?? 0;
  if (lastReset >= resetTs - 1) return null;

  const key = trafficAuditKey(h);
  let state = trafficAnchorState.get(key);
  if (!state || state.resetTs !== resetTs) {
    state = { resetTs, attempts: 0, nextAt: now };
    trafficAnchorState.set(key, state);
  }
  return { resetTs, state };
}

function dispatchAnchorAudit(h: HostItem, resetTs: number) {
  const key = trafficAuditKey(h);
  if (trafficAuditInFlight.has(key)) return;
  const state = trafficAnchorState.get(key);
  if (!state) return;

  state.attempts += 1;
  state.nextAt = Date.now() + TRAFFIC_ANCHOR_RETRY_MS;
  trafficAnchorInFlight.add(key);

  runHostTrafficAudit(h)
    .then((res) => {
      applyTrafficAuditResult(h, res);
      markTrafficAuditOutcome(h, res);
      if (res.traffic_source_effective !== "none" && res.last_reset_timestamp >= resetTs - 1) {
        trafficAnchorState.delete(key);
      }
      evaluateHostAlerts();
      void persistTrafficConfig();
    })
    .catch((err) => {
      const failures = (trafficAuditFailures.get(key) || 0) + 1;
      trafficAuditFailures.set(key, failures);
      const st = trafficAnchorState.get(key);
      if (st) st.nextAt = Date.now() + TRAFFIC_ANCHOR_RETRY_MS;
      console.warn(`[traffic-anchor] ${h.name} 锚点采样失败:`, err);
    })
    .finally(() => {
      trafficAnchorInFlight.delete(key);
    });
}

async function runTrafficAuditScheduler() {
  if (!trafficAuditSchedulerStarted || trafficAuditSchedulerBusy) return;
  trafficAuditSchedulerBusy = true;
  try {
    const hosts = configData.value.managed_hosts || [];
    const now = Date.now();
    const liveKeys = new Set(hosts.map(trafficAuditKey));

    for (const key of Array.from(trafficAuditNextAt.keys())) {
      if (!liveKeys.has(key)) {
        trafficAuditNextAt.delete(key);
        trafficAuditFailures.delete(key);
      }
    }

    for (const h of hosts) {
      const key = trafficAuditKey(h);
      if (!trafficAuditNextAt.has(key)) {
        trafficAuditNextAt.set(
          key,
          now + Math.floor(Math.random() * TRAFFIC_AUDIT_STARTUP_JITTER_MS)
        );
      }
    }

    for (const key of Array.from(trafficAnchorState.keys())) {
      if (!liveKeys.has(key)) trafficAnchorState.delete(key);
    }

    // 重置锚点优先：在窗口内每 5 秒重试，成功后立即停止
    const anchorCandidates: Array<{ h: HostItem; resetTs: number }> = [];
    for (const h of hosts) {
      const key = trafficAuditKey(h);
      const anchor = anchorWindowFor(h, now);
      if (!anchor) continue;
      if (anchor.state.attempts >= TRAFFIC_ANCHOR_MAX_ATTEMPTS) continue;
      if (anchor.state.nextAt > now || trafficAuditInFlight.has(key)) continue;
      anchorCandidates.push({ h, resetTs: anchor.resetTs });
    }
    anchorCandidates.sort((a, b) => a.resetTs - b.resetTs);

    for (const item of anchorCandidates) {
      if (trafficAnchorInFlight.size >= TRAFFIC_ANCHOR_MAX_CONCURRENCY) break;
      if (item.h.details) item.h.details.traffic_force_nic = false;
      dispatchAnchorAudit(item.h, item.resetTs);
    }

    const normalInFlight = Array.from(trafficAuditInFlight.keys()).filter(
      (key) => !trafficAnchorInFlight.has(key)
    ).length;
    if (normalInFlight >= TRAFFIC_AUDIT_MAX_CONCURRENCY) return;

    const dueHosts = hosts
      .filter((h) => {
        const key = trafficAuditKey(h);
        if (trafficAuditInFlight.has(key)) return false;
        const anchor = anchorWindowFor(h, now);
        if (anchor && anchor.state.attempts < TRAFFIC_ANCHOR_MAX_ATTEMPTS) {
          // 锚点窗口内由高优先级队列处理，普通审计不抢连接
          return false;
        }
        return (trafficAuditNextAt.get(key) ?? 0) <= now;
      })
      .sort(
        (a, b) =>
          (a.details?.traffic_last_sync_ts || 0) -
          (b.details?.traffic_last_sync_ts || 0)
      );

    let normalDispatched = normalInFlight;
    for (const h of dueHosts) {
      if (normalDispatched >= TRAFFIC_AUDIT_MAX_CONCURRENCY) break;
      dispatchScheduledTrafficAudit(h);
      normalDispatched += 1;
    }
  } finally {
    trafficAuditSchedulerBusy = false;
  }
}

function startTrafficAuditScheduler() {
  if (!isTauri || trafficAuditSchedulerStarted) return;
  trafficAuditSchedulerStarted = true;
  const now = Date.now();
  for (const h of configData.value.managed_hosts || []) {
    trafficAuditNextAt.set(
      trafficAuditKey(h),
      now + Math.floor(Math.random() * TRAFFIC_AUDIT_STARTUP_JITTER_MS)
    );
  }
  trafficAuditTimer = window.setInterval(() => {
    void runTrafficAuditScheduler();
  }, TRAFFIC_AUDIT_TICK_MS);
  window.setTimeout(() => {
    void runTrafficAuditScheduler();
  }, 1_200);
}

function stopTrafficAuditScheduler() {
  if (trafficAuditTimer !== null) {
    clearInterval(trafficAuditTimer);
    trafficAuditTimer = null;
  }
  trafficAuditSchedulerStarted = false;
}

function handleTrafficSchedulerWake() {
  if (document.visibilityState === "hidden") return;
  void runTrafficAuditScheduler();
}

async function syncTrafficStatsNow() {
  const m = trafficStatsModal.value;
  const h = m.host;
  if (!h || m.isSyncing) return;

  if (m.source === "custom" && !m.customCommand.trim()) {
    m.syncError = true;
    m.syncMessage = "选择自定义命令时，请填写远程执行命令。";
    return;
  }

  const d = ensureHostDetails(h);
  d.traffic_source = m.source;
  d.traffic_custom_command = m.customCommand;
  d.reset_day = m.resetDay;

  m.isSyncing = true;
  m.syncError = false;
  m.syncMessage = "正在连接服务器，获取流量数据...";

  try {
    const res = await runHostTrafficAudit(h);
    applyTrafficAuditResult(h, res);
    markTrafficAuditOutcome(h, res);
    m.calibUsed = ((d.traffic_cycle_used_bytes ?? res.traffic_cycle_used_bytes) / trafficUnitMultiplier(m.calibUnit)).toFixed(2);

    if (res.traffic_source_effective === "none") {
      m.syncError = true;
      m.syncMessage = res.traffic_source_error || "获取失败，未收到有效的流量数据。";
    } else {
      m.syncMessage = `获取成功 · 来源: ${trafficSourceLabel(res.traffic_source_effective)} · 当前周期已用 ${d.traffic_used || "0"}${d.traffic_used_unit || "GB"}`;
      if (res.traffic_source_error) {
        m.syncMessage += ` · ${res.traffic_source_error}`;
      }
    }

    await persistTrafficConfig();
    evaluateHostAlerts();
  } catch (err) {
    const failures = (trafficAuditFailures.get(trafficAuditKey(h)) || 0) + 1;
    trafficAuditFailures.set(trafficAuditKey(h), failures);
    trafficAuditNextAt.set(trafficAuditKey(h), Date.now() + trafficAuditBackoff(failures));
    m.syncError = true;
    m.syncMessage = `获取失败: ${String(err)}`;
  } finally {
    m.isSyncing = false;
  }
}

// 复制文本到剪贴板（与终端复制一致走原生命令，失败静默不影响菜单操作）
function copyText(str: string) {
  void writeClipboardText(str).catch((e) => {
    console.warn("[clipboard] copy failed:", e);
  });
  closeContextMenu();
}

// 重置流量为 0 (供主机管理大盘面板调用)
async function resetTrafficCounter(h: HostItem) {
  const d = ensureHostDetails(h);
  d.traffic_used = "0";
  d.traffic_cycle_used_bytes = 0;
  d.traffic_delta_bytes = 0;
  d.last_raw_bytes = 0;
  d.traffic_counter_sig = "";
  d.traffic_boot_id = "";
  d.traffic_iface_baseline = "";
  d.traffic_source_error = "";
  d.traffic_manual_override = false;
  trafficAuditFailures.delete(trafficAuditKey(h));
  trafficAuditNextAt.set(trafficAuditKey(h), Date.now() + trafficAuditIntervalFor(h));
  await persistTrafficConfig();
  evaluateHostAlerts();
  closeContextMenu();
}

// --- 设备列表与命令备忘录拖拽排序核心逻辑 ---
interface DragState {
  type: 'host' | 'device_group' | 'memo_item' | 'memo_group' | null;
  sourceGroupIdx: number | null;
  sourceItemIdx: number | null;
  targetGroupIdx: number | null;
  targetItemIdx: number | null;
  targetPosition: 'before' | 'after' | 'inside' | null;
}

const dragState = ref<DragState>({
  type: null,
  sourceGroupIdx: null,
  sourceItemIdx: null,
  targetGroupIdx: null,
  targetItemIdx: null,
  targetPosition: null,
});

function resetDragState() {
  dragState.value = {
    type: null,
    sourceGroupIdx: null,
    sourceItemIdx: null,
    targetGroupIdx: null,
    targetItemIdx: null,
    targetPosition: null,
  };
}

// Native WebView2 file-drop and HTML5 drag are mutually exclusive on Windows.
// Use pointer-based sorting for the internal host/memo lists so external file
// drop can stay enabled for SFTP.
type PointerDragKind = 'host' | 'device_group' | 'memo_item' | 'memo_group' | 'tab';
interface PointerDrag {
  kind: PointerDragKind;
  sourceGroup: number;
  sourceItem: number | null;
  startX: number;
  startY: number;
  active: boolean;
}
let pointerDrag: PointerDrag | null = null;
let suppressClickUntil = 0;

function beginPointerDrag(kind: PointerDragKind, sourceGroup: number, sourceItem: number | null, event: PointerEvent) {
  if (event.button !== 0) return;
  if (kind === 'tab' && (event.target as Element | null)?.closest('.custom-tab-btn')) return;
  pointerDrag = {
    kind,
    sourceGroup,
    sourceItem,
    startX: event.clientX,
    startY: event.clientY,
    active: false,
  };
  (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
}

function updatePointerDragTarget(clientX: number, clientY: number) {
  if (!pointerDrag) return;
  const el = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
  if (!el) return;

  if (pointerDrag.kind === 'tab') {
    const tabEl = el.closest('[data-tab-index]') as HTMLElement | null;
    if (tabEl) {
      dragOverTabIndex.value = Number(tabEl.dataset.tabIndex);
    } else {
      // 指针离开标签区必须清空落点，否则松手会按"最后经过的标签"排序。
      dragOverTabIndex.value = null;
    }
    return;
  }

  if (pointerDrag.kind === 'host') {
    const hostEl = el.closest('[data-host-index]') as HTMLElement | null;
    if (hostEl) {
      const groupIdx = Number(hostEl.dataset.hostGroup);
      const itemIdx = Number(hostEl.dataset.hostIndex);
      const rect = hostEl.getBoundingClientRect();
      dragState.value.targetGroupIdx = groupIdx;
      dragState.value.targetItemIdx = itemIdx;
      dragState.value.targetPosition = clientY < rect.top + rect.height / 2 ? 'before' : 'after';
      return;
    }
    const groupEl = el.closest('[data-device-group]') as HTMLElement | null;
    if (groupEl) {
      dragState.value.targetGroupIdx = Number(groupEl.dataset.deviceGroup);
      dragState.value.targetItemIdx = null;
      dragState.value.targetPosition = 'inside';
    }
    return;
  }

  if (pointerDrag.kind === 'device_group') {
    const groupEl = el.closest('[data-device-group]') as HTMLElement | null;
    if (groupEl) {
      const groupIdx = Number(groupEl.dataset.deviceGroup);
      const rect = groupEl.getBoundingClientRect();
      dragState.value.targetGroupIdx = groupIdx;
      dragState.value.targetItemIdx = null;
      dragState.value.targetPosition = clientY < rect.top + rect.height / 2 ? 'before' : 'after';
    }
    return;
  }

  if (pointerDrag.kind === 'memo_item') {
    const memoEl = el.closest('[data-memo-index]') as HTMLElement | null;
    if (memoEl) {
      const rect = memoEl.getBoundingClientRect();
      dragState.value.targetGroupIdx = Number(memoEl.dataset.memoGroup);
      dragState.value.targetItemIdx = Number(memoEl.dataset.memoIndex);
      dragState.value.targetPosition = clientY < rect.top + rect.height / 2 ? 'before' : 'after';
      return;
    }
    const groupEl = el.closest('[data-memo-group]') as HTMLElement | null;
    if (groupEl) {
      dragState.value.targetGroupIdx = Number(groupEl.dataset.memoGroup);
      dragState.value.targetItemIdx = null;
      dragState.value.targetPosition = 'inside';
    }
    return;
  }

  if (pointerDrag.kind === 'memo_group') {
    const groupEl = el.closest('[data-memo-group]') as HTMLElement | null;
    if (groupEl) {
      const rect = groupEl.getBoundingClientRect();
      dragState.value.targetGroupIdx = Number(groupEl.dataset.memoGroup);
      dragState.value.targetItemIdx = null;
      dragState.value.targetPosition = clientY < rect.top + rect.height / 2 ? 'before' : 'after';
    }
  }
}

function fakeDragEvent(): DragEvent {
  return {
    preventDefault() {},
    stopPropagation() {},
    dataTransfer: null,
  } as unknown as DragEvent;
}

let autoScrollRaf: number | null = null;
let autoScrollEl: HTMLElement | null = null;
let autoScrollVelocity = 0;
let autoScrollPointerX = 0;
let autoScrollPointerY = 0;

function stopPointerAutoScroll() {
  if (autoScrollRaf !== null) {
    cancelAnimationFrame(autoScrollRaf);
    autoScrollRaf = null;
  }
  autoScrollVelocity = 0;
  autoScrollEl = null;
}

function pointerAutoScrollTick() {
  autoScrollRaf = null;
  if (!pointerDrag?.active || !autoScrollEl || autoScrollVelocity === 0) {
    stopPointerAutoScroll();
    return;
  }
  const before = autoScrollEl.scrollTop;
  autoScrollEl.scrollTop += autoScrollVelocity;
  // Re-evaluate the drop target while the list moves under the cursor.
  if (autoScrollEl.scrollTop !== before) {
    updatePointerDragTarget(autoScrollPointerX, autoScrollPointerY);
  }
  autoScrollRaf = requestAnimationFrame(pointerAutoScrollTick);
}

function updatePointerAutoScroll(clientX: number, clientY: number) {
  if (!pointerDrag?.active || pointerDrag.kind === 'tab') {
    stopPointerAutoScroll();
    return;
  }

  const container = pointerDrag.kind === 'host' || pointerDrag.kind === 'device_group'
    ? (document.querySelector('.tree-scroll') as HTMLElement | null)
    : (document.querySelector('.memo-container') as HTMLElement | null);

  if (!container || container.scrollHeight <= container.clientHeight) {
    stopPointerAutoScroll();
    return;
  }

  const rect = container.getBoundingClientRect();
  const insideX = clientX >= rect.left && clientX <= rect.right;
  if (!insideX) {
    stopPointerAutoScroll();
    return;
  }

  const edge = 42;
  const maxSpeed = 14;
  let velocity = 0;
  if (clientY >= rect.top && clientY <= rect.top + edge) {
    velocity = -Math.ceil(((rect.top + edge - clientY) / edge) * maxSpeed);
  } else if (clientY >= rect.bottom - edge && clientY <= rect.bottom) {
    velocity = Math.ceil(((clientY - (rect.bottom - edge)) / edge) * maxSpeed);
  }

  if (velocity === 0) {
    stopPointerAutoScroll();
    return;
  }

  autoScrollEl = container;
  autoScrollVelocity = velocity;
  autoScrollPointerX = clientX;
  autoScrollPointerY = clientY;
  if (autoScrollRaf === null) {
    autoScrollRaf = requestAnimationFrame(pointerAutoScrollTick);
  }
}

function onPointerDragMove(event: PointerEvent) {
  const drag = pointerDrag;
  if (!drag) return;
  if (!drag.active) {
    if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 5) return;
    drag.active = true;
    document.body.style.userSelect = 'none';
    if (drag.kind === 'tab') {
      draggingTabIndex.value = drag.sourceGroup;
    } else {
      dragState.value = {
        type: drag.kind,
        sourceGroupIdx: drag.sourceGroup,
        sourceItemIdx: drag.sourceItem,
        targetGroupIdx: null,
        targetItemIdx: null,
        targetPosition: null,
      };
    }
  }
  updatePointerDragTarget(event.clientX, event.clientY);
  updatePointerAutoScroll(event.clientX, event.clientY);
}

function onPointerDragEnd(event: PointerEvent) {
  const drag = pointerDrag;
  if (!drag) return;
  stopPointerAutoScroll();
  pointerDrag = null;
  document.body.style.userSelect = '';

  if (!drag.active) {
    resetDragState();
    return;
  }

  if (drag.kind === 'tab') {
    suppressClickUntil = performance.now() + 350;
    if (dragOverTabIndex.value !== null) {
      handleTabDrop(dragOverTabIndex.value, fakeDragEvent());
    }
    draggingTabIndex.value = null;
    dragOverTabIndex.value = null;
    resetDragState();
    return;
  }

  const target = dragState.value.targetGroupIdx;
  suppressClickUntil = performance.now() + 350;
  const fake = fakeDragEvent();

  if (target !== null) {
    if (drag.kind === 'host') {
      void onHostDrop(target, dragState.value.targetItemIdx, fake);
    } else if (drag.kind === 'device_group') {
      void onDeviceGroupDrop(target, fake);
    } else if (drag.kind === 'memo_item') {
      void onMemoDrop(target, dragState.value.targetItemIdx, fake);
    } else if (drag.kind === 'memo_group') {
      void onMemoGroupDrop(target, fake);
    }
  } else {
    resetDragState();
  }

  (event.currentTarget as HTMLElement | null)?.releasePointerCapture?.(event.pointerId);
}

function onPointerDragCancel() {
  stopPointerAutoScroll();
  pointerDrag = null;
  draggingTabIndex.value = null;
  dragOverTabIndex.value = null;
  document.body.style.userSelect = '';
  resetDragState();
}

function suppressClickAfterPointerDrag(event: MouseEvent) {
  if (performance.now() < suppressClickUntil) {
    event.preventDefault();
    event.stopPropagation();
  }
}

// 1. 设备主机拖拽
function onHostDragStart(gIdx: number, hIdx: number, e: DragEvent) {
  dragState.value.type = 'host';
  dragState.value.sourceGroupIdx = gIdx;
  dragState.value.sourceItemIdx = hIdx;
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', `host:${gIdx}:${hIdx}`);
  }
}

function onHostDragOver(gIdx: number, hIdx: number, e: DragEvent) {
  if (dragState.value.type !== 'host') return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';

  const target = e.currentTarget as HTMLElement | null;
  if (!target) return;
  const rect = target.getBoundingClientRect();
  const relY = e.clientY - rect.top;
  const pos: 'before' | 'after' = relY < rect.height / 2 ? 'before' : 'after';

  dragState.value.targetGroupIdx = gIdx;
  dragState.value.targetItemIdx = hIdx;
  dragState.value.targetPosition = pos;
}

async function onHostDrop(targetGroupIdx: number, targetHostIdx: number | null, e: DragEvent) {
  e.preventDefault();
  e.stopPropagation();

  if (dragState.value.type !== 'host' || dragState.value.sourceGroupIdx === null || dragState.value.sourceItemIdx === null) {
    resetDragState();
    return;
  }

  const srcGIdx = dragState.value.sourceGroupIdx;
  const srcHIdx = dragState.value.sourceItemIdx;
  const srcGroup = configData.value.devices[srcGIdx];
  const tgtGroup = configData.value.devices[targetGroupIdx];

  if (!srcGroup || !tgtGroup) {
    resetDragState();
    return;
  }

  // 取出移动的主机
  const [movedHost] = srcGroup.hosts.splice(srcHIdx, 1);
  if (!movedHost) {
    resetDragState();
    return;
  }

  if (targetHostIdx === null) {
    // 拖放到分组空白或分组头上
    tgtGroup.hosts.push(movedHost);
  } else {
    // 拖放到某具体主机前后
    let insertIdx = targetHostIdx;
    // 如果同一分组且原位置在目标之前，则由于 splice(1) 原数组长度减 1，需要校准 insertIdx
    if (srcGIdx === targetGroupIdx && srcHIdx < targetHostIdx) {
      insertIdx--;
    }
    if (dragState.value.targetPosition === 'after') {
      insertIdx++;
    }
    if (insertIdx < 0) insertIdx = 0;
    if (insertIdx > tgtGroup.hosts.length) insertIdx = tgtGroup.hosts.length;
    tgtGroup.hosts.splice(insertIdx, 0, movedHost);
  }

  resetDragState();
  await invoke("save_hosts_config", { config: configData.value });
}

// 2. 设备分组拖拽
function onDeviceGroupDragStart(gIdx: number, e: DragEvent) {
  dragState.value.type = 'device_group';
  dragState.value.sourceGroupIdx = gIdx;
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', `device_group:${gIdx}`);
  }
}

function onDeviceGroupDragOver(gIdx: number, e: DragEvent) {
  if (dragState.value.type === 'host') {
    // 允许将主机拖拽放置到分组标题头上
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragState.value.targetGroupIdx = gIdx;
    dragState.value.targetItemIdx = null;
    dragState.value.targetPosition = 'inside';
    return;
  }
  if (dragState.value.type !== 'device_group') return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';

  const target = e.currentTarget as HTMLElement | null;
  if (!target) return;
  const rect = target.getBoundingClientRect();
  const relY = e.clientY - rect.top;
  const pos: 'before' | 'after' = relY < rect.height / 2 ? 'before' : 'after';

  dragState.value.targetGroupIdx = gIdx;
  dragState.value.targetPosition = pos;
}

async function onDeviceGroupDrop(targetGroupIdx: number, e: DragEvent) {
  e.preventDefault();
  e.stopPropagation();

  if (dragState.value.type === 'host') {
    await onHostDrop(targetGroupIdx, null, e);
    return;
  }

  if (dragState.value.type !== 'device_group' || dragState.value.sourceGroupIdx === null) {
    resetDragState();
    return;
  }

  const srcGIdx = dragState.value.sourceGroupIdx;
  if (srcGIdx === targetGroupIdx) {
    resetDragState();
    return;
  }

  const [movedGroup] = configData.value.devices.splice(srcGIdx, 1);
  if (movedGroup) {
    let insertIdx = targetGroupIdx;
    if (srcGIdx < targetGroupIdx) {
      insertIdx--;
    }
    if (dragState.value.targetPosition === 'after') {
      insertIdx++;
    }
    if (insertIdx < 0) insertIdx = 0;
    if (insertIdx > configData.value.devices.length) insertIdx = configData.value.devices.length;
    configData.value.devices.splice(insertIdx, 0, movedGroup);
    await invoke("save_hosts_config", { config: configData.value });
  }

  resetDragState();
}

// 3. 命令备忘录条目拖拽
function onMemoDragStart(gIdx: number, iIdx: number, e: DragEvent) {
  dragState.value.type = 'memo_item';
  dragState.value.sourceGroupIdx = gIdx;
  dragState.value.sourceItemIdx = iIdx;
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', `memo_item:${gIdx}:${iIdx}`);
  }
}

function onMemoDragOver(gIdx: number, iIdx: number, e: DragEvent) {
  if (dragState.value.type !== 'memo_item') return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';

  const target = e.currentTarget as HTMLElement | null;
  if (!target) return;
  const rect = target.getBoundingClientRect();
  const relY = e.clientY - rect.top;
  const pos: 'before' | 'after' = relY < rect.height / 2 ? 'before' : 'after';

  dragState.value.targetGroupIdx = gIdx;
  dragState.value.targetItemIdx = iIdx;
  dragState.value.targetPosition = pos;
}

async function onMemoDrop(targetGroupIdx: number, targetItemIdx: number | null, e: DragEvent) {
  e.preventDefault();
  e.stopPropagation();

  if (dragState.value.type !== 'memo_item' || dragState.value.sourceGroupIdx === null || dragState.value.sourceItemIdx === null) {
    resetDragState();
    return;
  }

  const srcGIdx = dragState.value.sourceGroupIdx;
  const srcIIdx = dragState.value.sourceItemIdx;
  const srcGroup = configData.value.memos?.[srcGIdx];
  const tgtGroup = configData.value.memos?.[targetGroupIdx];

  if (!srcGroup || !tgtGroup) {
    resetDragState();
    return;
  }

  const [movedItem] = srcGroup.items.splice(srcIIdx, 1);
  if (!movedItem) {
    resetDragState();
    return;
  }

  if (targetItemIdx === null) {
    tgtGroup.items.push(movedItem);
  } else {
    let insertIdx = targetItemIdx;
    if (srcGIdx === targetGroupIdx && srcIIdx < targetItemIdx) {
      insertIdx--;
    }
    if (dragState.value.targetPosition === 'after') {
      insertIdx++;
    }
    if (insertIdx < 0) insertIdx = 0;
    if (insertIdx > tgtGroup.items.length) insertIdx = tgtGroup.items.length;
    tgtGroup.items.splice(insertIdx, 0, movedItem);
  }

  resetDragState();
  await invoke("save_hosts_config", { config: configData.value });
}

// 4. 命令备忘录分组拖拽
function onMemoGroupDragStart(gIdx: number, e: DragEvent) {
  dragState.value.type = 'memo_group';
  dragState.value.sourceGroupIdx = gIdx;
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', `memo_group:${gIdx}`);
  }
}

function onMemoGroupDragOver(gIdx: number, e: DragEvent) {
  if (dragState.value.type === 'memo_item') {
    // 允许条目拖到备忘录分组头上直接推入
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragState.value.targetGroupIdx = gIdx;
    dragState.value.targetItemIdx = null;
    dragState.value.targetPosition = 'inside';
    return;
  }
  if (dragState.value.type !== 'memo_group') return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';

  const target = e.currentTarget as HTMLElement | null;
  if (!target) return;
  const rect = target.getBoundingClientRect();
  const relY = e.clientY - rect.top;
  const pos: 'before' | 'after' = relY < rect.height / 2 ? 'before' : 'after';

  dragState.value.targetGroupIdx = gIdx;
  dragState.value.targetPosition = pos;
}

async function onMemoGroupDrop(targetGroupIdx: number, e: DragEvent) {
  e.preventDefault();
  e.stopPropagation();

  if (dragState.value.type === 'memo_item') {
    await onMemoDrop(targetGroupIdx, null, e);
    return;
  }

  if (dragState.value.type !== 'memo_group' || dragState.value.sourceGroupIdx === null) {
    resetDragState();
    return;
  }

  const srcGIdx = dragState.value.sourceGroupIdx;
  if (srcGIdx === targetGroupIdx || !configData.value.memos) {
    resetDragState();
    return;
  }

  const [movedGroup] = configData.value.memos.splice(srcGIdx, 1);
  if (movedGroup) {
    let insertIdx = targetGroupIdx;
    if (srcGIdx < targetGroupIdx) {
      insertIdx--;
    }
    if (dragState.value.targetPosition === 'after') {
      insertIdx++;
    }
    if (insertIdx < 0) insertIdx = 0;
    if (insertIdx > configData.value.memos.length) insertIdx = configData.value.memos.length;
    configData.value.memos.splice(insertIdx, 0, movedGroup);
    await invoke("save_hosts_config", { config: configData.value });
  }

  resetDragState();
}

// 分组新建、重命名与删除操作
function openNewGroupModal() {
  closeContextMenu();
  promptModal.value = {
    show: true,
    title: "新建设备分组",
    label: "分组名称:",
    value: "新分组",
    action: "new_device_group",
    payload: null,
  };
}

function openRenameGroupModal(g: DeviceGroup) {
  closeContextMenu();
  promptModal.value = {
    show: true,
    title: `重命名设备分组: ${g.group}`,
    label: "新分组名称:",
    value: g.group,
    action: "rename_device_group",
    payload: { group: g },
  };
}

async function deleteGroup(g: DeviceGroup) {
  closeContextMenu();
  configData.value.devices = configData.value.devices.filter(item => item !== g);
  await invoke("save_hosts_config", { config: configData.value });
}

function openNewMemoGroupModal() {
  closeContextMenu();
  promptModal.value = {
    show: true,
    title: "新建备忘录分组",
    label: "分组名称:",
    value: "新备忘组",
    action: "new_memo_group",
    payload: null,
  };
}

function openRenameMemoGroupModal(groupIdx: number) {
  closeContextMenu();
  const group = configData.value.memos?.[groupIdx];
  if (!group) return;
  promptModal.value = {
    show: true,
    title: `重命名备忘录分组: ${group.group}`,
    label: "新分组名称:",
    value: group.group,
    action: "rename_memo_group",
    payload: { groupIdx },
  };
}

async function deleteMemoGroup(groupIdx: number) {
  closeContextMenu();
  if (!configData.value.memos) return;
  configData.value.memos.splice(groupIdx, 1);
  await invoke("save_hosts_config", { config: configData.value });
}

// --- 配置加密（主密码，方案 A：PBKDF2 + AES-256-GCM）---
const configEncryptionEnabled = ref(false);
const configLocked = ref(false);
const unlockModalShow = ref(false);
const unlockBusy = ref(false);
const unlockError = ref("");
const encryptionModalShow = ref(false);
const encryptionModalMode = ref<"enable" | "disable">("enable");
const encryptionModalBusy = ref(false);
const encryptionModalError = ref("");

async function refreshConfigEncryptionState() {
  if (!isTauri) return;
  try {
    configEncryptionEnabled.value = await invoke<boolean>("is_config_encrypted");
  } catch (e) {
    console.warn("[config] 读取加密状态失败:", e);
  }
}

function openEncryptionModal(mode: "enable" | "disable") {
  encryptionModalMode.value = mode;
  encryptionModalError.value = "";
  encryptionModalShow.value = true;
}

async function submitEncryptionChange(payload: { password: string; confirm: string }) {
  const { password, confirm } = payload;
  const mode = encryptionModalMode.value;
  if (!password) {
    encryptionModalError.value = "请输入主密码";
    return;
  }
  if (mode === "enable" && password !== confirm) {
    encryptionModalError.value = "两次输入的密码不一致";
    return;
  }
  if (mode === "enable" && password.length < 4) {
    encryptionModalError.value = "主密码至少 4 位";
    return;
  }
  encryptionModalBusy.value = true;
  encryptionModalError.value = "";
  try {
    if (mode === "enable") {
      // 先把当前内存里的最新配置落盘，再整体加密
      await invoke("save_hosts_config", { config: configData.value });
    }
    await invoke("set_config_encryption", { enabled: mode === "enable", password });
    configEncryptionEnabled.value = mode === "enable";
    encryptionModalShow.value = false;
  } catch (e: any) {
    encryptionModalError.value = String(e?.message || e);
  } finally {
    encryptionModalBusy.value = false;
  }
}

async function submitUnlock(password: string) {
  if (!password) {
    unlockError.value = "请输入主密码";
    return;
  }
  unlockBusy.value = true;
  unlockError.value = "";
  try {
    await invoke("unlock_config", { password });
    unlockModalShow.value = false;
    configLocked.value = false;
    await loadConfig();
  } catch (e: any) {
    unlockError.value = String(e?.message || e);
  } finally {
    unlockBusy.value = false;
  }
}

// --- 自动更新 ---
const updating = ref(false);

async function checkForUpdates(silent = false) {
  if (!isTauri) return;
  try {
    const update = await check();
    if (!update) {
      if (!silent) await message("当前已是最新版本。", { title: "检查更新", kind: "info" });
      return;
    }
    const yes = await ask(
      `发现新版本 ${update.version}（当前 ${update.currentVersion}）。\n\n${update.body || ""}\n\n是否下载并安装？`,
      { title: "发现新版本", kind: "info" }
    );
    if (!yes) return;
    updating.value = true;
    await update.downloadAndInstall();
    await relaunch();
  } catch (e: any) {
    if (!silent) await message(`检查更新失败：${e?.message || e}`, { title: "检查更新", kind: "error" });
  } finally {
    updating.value = false;
  }
}

// --- 配置加载 (支持在浏览器直接开发预览与调试) ---
async function loadConfig() {
  try {
    let cfg: AppConfig;
    if (isTauri) {
      try {
        cfg = await invoke<AppConfig>("load_hosts_config");
      } catch (lockError: any) {
        if (String(lockError).includes("CONFIG_LOCKED")) {
          // 配置已加密且尚未解锁：弹出解锁框，解锁成功后会重新 loadConfig
          configLocked.value = true;
          unlockModalShow.value = true;
          return;
        }
        throw lockError;
      }
    } else {
      const localStr = localStorage.getItem("qshell_mock_config");
      if (localStr) {
        cfg = JSON.parse(localStr);
      } else {
        cfg = {
          devices: [
            {
              group: "VPS",
              hosts: [
                { name: "东京 Acck", host: "192.168.1.10", port: 22, user: "root" },
                { name: "香港 Hytron", host: "192.168.1.11", port: 22, user: "root" },
                { name: "英国 Akile", host: "192.168.1.12", port: 22, user: "root" },
              ]
            },
            {
              group: "本地",
              hosts: [
                { name: "OpenWrt", host: "192.168.254.1", port: 2281, user: "root" },
                { name: "Debian(新城)", host: "192.168.1.2", port: 22, user: "root" },
              ]
            }
          ],
          managed_hosts: [],
          memos: [
            {
              group: "系统运维常用",
              items: [
                { name: "查看监听端口", cmd: "netstat -tulnp" },
                { name: "查看内存 Top", cmd: "ps aux --sort=-%mem | head -n 10" },
                { name: "系统日志", cmd: "journalctl -xe --no-pager | tail -n 20" },
                { name: "系统负载状态", cmd: "htop" },
                { name: "Docker 容器状态", cmd: "docker ps -a" },
              ]
            }
          ],
          settings: {
            font_family: "Consolas",
            font_size: 11,
            is_bold: false,
            cursor_style: "block",
            cursor_blink: true,
            terminal_fg_color: "#81C995",
          }
        };
      }
    }
    configData.value = cfg;
    if (cfg.settings) {
      if (cfg.settings.app_font_family) {
        applyAppFont(cfg.settings.app_font_family);
      }
      if (cfg.settings.font_family) terminalSettings.value.font_family = cfg.settings.font_family;
      if (cfg.settings.font_size) terminalSettings.value.font_size = cfg.settings.font_size;
      if (cfg.settings.is_bold !== undefined) terminalSettings.value.is_bold = !!cfg.settings.is_bold;
      if (cfg.settings.cursor_style) terminalSettings.value.cursor_style = cfg.settings.cursor_style as any;
      if (cfg.settings.cursor_blink !== undefined) terminalSettings.value.cursor_blink = !!cfg.settings.cursor_blink;
      if (cfg.settings.blink_interval) terminalSettings.value.blink_interval = Number(cfg.settings.blink_interval) || 530;
      const storedBgMode = cfg.settings.terminal_bg_mode || cfg.settings.term_bg_mode;
      if (storedBgMode) terminalSettings.value.term_bg_mode = storedBgMode as any;
      if (cfg.settings.terminal_fg_color) terminalSettings.value.terminal_fg_color = cfg.settings.terminal_fg_color;
      if (cfg.settings.terminal_bg_color) terminalSettings.value.terminal_bg_color = cfg.settings.terminal_bg_color;
      if (cfg.settings.minimize_to_tray !== undefined) terminalSettings.value.minimize_to_tray = !!cfg.settings.minimize_to_tray;
      if (cfg.settings.accent_color) terminalSettings.value.accent_color = cfg.settings.accent_color;
      if (cfg.settings.terminal_bg_image_fit) {
        terminalSettings.value.bg_image_fit = cfg.settings.terminal_bg_image_fit;
      }
      if (cfg.settings.terminal_bg_image_dim !== undefined) {
        terminalSettings.value.bg_image_dim = Number(cfg.settings.terminal_bg_image_dim) || 0;
      }
      if (cfg.settings.terminal_bg_image_auto_fg !== undefined) {
        terminalSettings.value.bg_image_auto_fg = !!cfg.settings.terminal_bg_image_auto_fg;
      }
      const storedBgImage = cfg.settings.terminal_bg_image || cfg.settings.custom_bg_image;
      if (storedBgImage) {
        await setTerminalBackgroundImage(storedBgImage, false);
      }
      if (cfg.settings.webdav) {
        backupModal.value.webdav_url = cfg.settings.webdav.url || "";
        backupModal.value.webdav_user = cfg.settings.webdav.user || "";
        backupModal.value.webdav_pwd = cfg.settings.webdav.pwd || "";
        const storedWebdavPath = (cfg.settings.webdav.path || "").trim();
        backupModal.value.webdav_path =
          !storedWebdavPath || storedWebdavPath === "/qshell_backup.json"
            ? "/QShell/qshell_backup.zip"
            : storedWebdavPath;
      }
    }
    if (!configData.value.memos || configData.value.memos.length === 0) {
      configData.value.memos = [
        {
          group: "系统运维常用",
          items: [
            { name: "查看监听端口", cmd: "netstat -tulnp" },
            { name: "查看内存 Top", cmd: "ps aux --sort=-%mem | head -n 10" },
            { name: "系统日志", cmd: "journalctl -xe --no-pager | tail -n 20" },
            { name: "系统负载状态", cmd: "htop" },
            { name: "Docker 容器状态", cmd: "docker ps -a" },
          ]
        }
      ];
    }
    evaluateHostAlerts();
  } catch (e) {
    console.error("加载配置失败:", e);
  }
}

// 1:1 对标 ssh8.py TerminalSettingsDialog: 设置终端背景预设 (黑/白)
function setTerminalBgPreset(mode: "dark" | "light") {
  terminalSettings.value.term_bg_mode = mode;
  if (mode === "dark") {
    terminalSettings.value.terminal_bg_color = "#0A0B0E";
    terminalSettings.value.terminal_fg_color = "#81C995";
  } else {
    terminalSettings.value.terminal_bg_color = "#F4F5F7";
    terminalSettings.value.terminal_fg_color = "#111318";
  }
}

// 1:1 对标 ssh8.py refresh_color_palette_buttons
const darkPresetColors = ["#81C995", "#A8C7FA", "#FDD663", "#E2E2E9"];
const lightPresetColors = ["#111318", "#0B57D0", "#15803D", "#44474E"];
const accentPresetColors = ["#A8C7FA", "#78D9EC", "#81C995", "#FDD663", "#D7AEFB", "#F28B82"];

function normalizeHexColor(value: string): string | null {
  const raw = String(value || "").trim();
  const short = /^#([0-9a-f]{3})$/i.exec(raw);
  if (short) {
    return "#" + short[1].split("").map((c) => c + c).join("");
  }
  return /^#[0-9a-f]{6}$/i.test(raw) ? raw : null;
}

function mixHexColor(from: string, to: string, amount: number): string {
  const a = parseInt(from.slice(1), 16);
  const b = parseInt(to.slice(1), 16);
  const t = Math.max(0, Math.min(1, amount));
  const channels = [16, 8, 0].map((shift) => {
    const av = (a >> shift) & 0xff;
    const bv = (b >> shift) & 0xff;
    return Math.round(av + (bv - av) * t);
  });
  return "#" + channels.map((v) => v.toString(16).padStart(2, "0")).join("");
}

function applyAccentColor(value?: string) {
  const root = document.documentElement;
  const color = normalizeHexColor(value || "");
  if (!color) {
    root.style.removeProperty("--btn-primary-bg");
    root.style.removeProperty("--btn-primary-hover");
    root.style.removeProperty("--btn-primary-active");
    root.style.removeProperty("--btn-primary-text");
    return;
  }
  const rgb = parseInt(color.slice(1), 16);
  const r = (rgb >> 16) & 0xff;
  const g = (rgb >> 8) & 0xff;
  const b = rgb & 0xff;
  const luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  const isLight = document.documentElement.getAttribute("data-theme") === "light";
  const text = luminance > 0.58 ? "#042E61" : "#FFFFFF";
  root.style.setProperty("--btn-primary-bg", color);
  root.style.setProperty(
    "--btn-primary-hover",
    mixHexColor(color, isLight ? "#000000" : "#FFFFFF", isLight ? 0.12 : 0.16),
  );
  root.style.setProperty("--btn-primary-active", mixHexColor(color, "#000000", isLight ? 0.2 : 0.14));
  root.style.setProperty("--btn-primary-text", text);
}

function setAccentColor(value: string) {
  terminalSettings.value.accent_color = value;
  applyAccentColor(value);
}

function resetAccentColor() {
  terminalSettings.value.accent_color = "";
  applyAccentColor("");
}

function openBackupFromSettings() {
  showTerminalSettingsModal.value = false;
  showWebdavPassword.value = false;
  backupModal.value.show = true;
}

// 1:1 对标 ssh8.py applyCursorBlinkSettings: 动态计算并在终端根容器注入闪烁动画与 CSS 变量
function applyCursorBlinkToTerminal(term: Terminal, enabled: boolean, intervalMs: number, fgColor: string, bgColor: string, cursorStyle: string) {
  if (!term.element) {
    console.warn("[cursor-blink] term.element is null!");
    return;
  }
  const phaseDuration = Math.max(200, Math.min(2000, Number(intervalMs) || 530));
  const cycleDuration = phaseDuration * 2;

  const rawEl = term.element as HTMLElement;
  console.log("[cursor-blink] rawEl:", rawEl.tagName, "classes:", rawEl.className);
  console.log("[cursor-blink] rawEl.parentElement:", rawEl.parentElement?.tagName, rawEl.parentElement?.className);

  // 查找带有 .xterm 类的元素
  const xtermEl = rawEl.classList.contains("xterm")
    ? rawEl
    : rawEl.querySelector(".xterm") as HTMLElement | null;
  const mountBox = rawEl.closest(".terminal-mount-box") || rawEl.parentElement;
  const fallbackXtermEl = mountBox?.querySelector(".xterm") as HTMLElement | null;
  const el = xtermEl || fallbackXtermEl || rawEl;

  console.log("[cursor-blink] final el:", el.tagName, "classes:", el.className);
  console.log("[cursor-blink] enabled:", enabled, "style:", cursorStyle);

  // 打印 cursor 元素的完整 DOM 路径
  const cursorNode = el.querySelector("[class*='xterm-cursor']");
  if (cursorNode) {
    const chain: string[] = [];
    let node = cursorNode as HTMLElement;
    while (node && node !== el) {
      chain.unshift(node.tagName + "." + Array.from(node.classList).join("."));
      node = node.parentElement as HTMLElement;
    }
    console.log("[cursor-blink] cursor DOM path:", chain.join(" > "));
  } else {
    console.log("[cursor-blink] cursor element NOT found yet (may render later)");
  }

  el.classList.toggle("ssh-cursor-blink-enabled", enabled);
  el.classList.toggle("ssh-cursor-blink-disabled", !enabled);
  el.style.setProperty("--ssh-cursor-blink-cycle", `${cycleDuration}ms`);
  el.style.setProperty("--ssh-cursor-color", fgColor || "#81C995");
  el.style.setProperty("--ssh-cursor-accent", bgColor || "#0A0B0E");
  el.style.setProperty("--ssh-cursor-bar-shadow", `1px 0 0 ${fgColor || '#81C995'} inset`);
  
  // 同步 xterm options
  term.options.cursorBlink = enabled;
  const normalizedStyle = (["block", "bar", "underline"].includes(cursorStyle) ? cursorStyle : "block") as "block" | "bar" | "underline";
  term.options.cursorStyle = normalizedStyle;
  term.options.cursorInactiveStyle = normalizedStyle;

  console.log("[cursor-blink] DONE - el classes now:", el.className);

  if (typeof (term as any).refresh === "function" && term.rows > 0) {
    term.refresh(0, term.rows - 1);
  }
}

// 应用终端设置到所有已激活的终端 (1:1 严格对标 ssh8.py refresh_all_terminal_appearances)
function terminalBackgroundStyle() {
  const data = terminalBgImageData.value;
  if (!data) return null;
  const dim = Math.max(0, Math.min(0.85, terminalSettings.value.bg_image_dim));
  const overlay = `linear-gradient(rgba(0,0,0,${dim}), rgba(0,0,0,${dim}))`;
  const fit = terminalSettings.value.bg_image_fit;
  const size = fit === "contain" ? "contain" : fit === "stretch" ? "100% 100%" : fit === "tile" ? "auto" : "cover";
  return {
    image: `${overlay}, url("${data}")`,
    size,
    repeat: fit === "tile" ? "repeat" : "no-repeat",
  };
}

function applyTerminalBackgroundToMount(mountEl: HTMLElement) {
  const style = terminalBackgroundStyle();
  if (!style) {
    mountEl.classList.remove("has-terminal-bg");
    mountEl.style.backgroundImage = "";
    mountEl.style.backgroundSize = "";
    mountEl.style.backgroundRepeat = "";
    return;
  }
  mountEl.classList.add("has-terminal-bg");
  mountEl.style.backgroundImage = style.image;
  mountEl.style.backgroundSize = style.size;
  mountEl.style.backgroundRepeat = style.repeat;
  mountEl.style.backgroundPosition = "center";
}

function applyTerminalBackgroundToAll() {
  sessions.value.forEach((session) => {
    const mountEl = document.getElementById(`term-${session.id}`);
    if (mountEl) applyTerminalBackgroundToMount(mountEl);
  });
}

function analyzeImageLuminance(dataUrl: string): Promise<number> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = 32;
      canvas.height = 32;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) {
        resolve(0.5);
        return;
      }
      ctx.drawImage(img, 0, 0, 32, 32);
      const pixels = ctx.getImageData(0, 0, 32, 32).data;
      let total = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        const r = pixels[i] / 255;
        const g = pixels[i + 1] / 255;
        const b = pixels[i + 2] / 255;
        const linear = (c: number) => c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
        total += 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
      }
      resolve(total / (pixels.length / 4));
    };
    img.onerror = () => resolve(0.5);
    img.src = dataUrl;
  });
}

async function chooseTerminalBackgroundImage() {
  const path = await openLocalFileBrowser({
    action: "open",
    title: "选择终端背景图",
  });
  if (!path) return;
  await setTerminalBackgroundImage(path);
}

async function setTerminalBackgroundImage(path: string, persist = true) {
  try {
    const data = await invoke<string>("read_local_image_data_url", { path });
    terminalSettings.value.custom_bg_image = path;
    terminalBgImageData.value = data;
    terminalBgImageLuminance.value = await analyzeImageLuminance(data);
    if (terminalSettings.value.bg_image_auto_fg) {
      // 黑色遮罩后的有效亮度决定用浅色还是深色文字。
      const effective = terminalBgImageLuminance.value * (1 - terminalSettings.value.bg_image_dim);
      terminalSettings.value.terminal_fg_color = effective < 0.45 ? "#FFFFFF" : "#111318";
    }
    if (persist) {
      applyTerminalSettings(false);
    } else {
      applyTerminalBackgroundToAll();
    }
  } catch (e: any) {
    console.error("[terminal-bg] 读取背景图失败:", e);
  }
}

function removeTerminalBackgroundImage() {
  terminalSettings.value.custom_bg_image = "";
  terminalBgImageData.value = "";
  terminalBgImageLuminance.value = 0;
  applyTerminalSettings(false);
}

function updateTerminalBackgroundPreview() {
  if (terminalSettings.value.bg_image_auto_fg && terminalBgImageData.value) {
    const effective = terminalBgImageLuminance.value * (1 - terminalSettings.value.bg_image_dim);
    terminalSettings.value.terminal_fg_color = effective < 0.45 ? "#FFFFFF" : "#111318";
  }
  applyTerminalBackgroundToAll();
}

function applyTerminalSettings(closeModal: boolean | MouseEvent = true) {
  const shouldClose = typeof closeModal === "boolean" ? closeModal : true;
  const isDark = terminalSettings.value.term_bg_mode === "dark";
  const bgCol = isDark ? "#0A0B0E" : "#F4F5F7";
  const hasBgImage = !!terminalBgImageData.value;
  const fgCol = hasBgImage
    ? terminalSettings.value.terminal_fg_color || (isDark ? "#81C995" : "#111318")
    : ensureReadableFg(
        terminalSettings.value.terminal_fg_color,
        bgCol,
        isDark ? "#81C995" : "#111318",
      );
  terminalSettings.value.terminal_bg_color = bgCol;
  terminalSettings.value.terminal_fg_color = fgCol;

  sessions.value.forEach(s => {
    if (s.termInstance) {
      const term = s.termInstance;
      term.options.fontFamily = terminalSettings.value.font_family;
      term.options.fontSize = terminalSettings.value.font_size;
      term.options.fontWeight = terminalSettings.value.is_bold ? "bold" : "normal";
      term.options.cursorStyle = terminalSettings.value.cursor_style;
      term.options.cursorInactiveStyle = terminalSettings.value.cursor_style;
      
      // 更新 xterm 调色板
      term.options.theme = {
        background: hasBgImage ? "rgba(0,0,0,0)" : bgCol,
        foreground: fgCol,
        cursor: fgCol,
        cursorAccent: bgCol,
        selectionBackground: isDark ? "rgba(168, 199, 250, 0.3)" : "#CBD5E1",
        black: isDark ? "#1A1D24" : "#000000",
        red: "#F28B82",
        green: "#81C995",
        yellow: "#FDD663",
        blue: isDark ? "#78D9EC" : "#1A73E8",
        magenta: "#D7AEFB",
        cyan: "#78D9EC",
        white: isDark ? "#FFFFFF" : "#1F2937",
      };

      // 同步外层挂载 DOM 的背景色
      const mountEl = document.getElementById(`term-${s.id}`);
      if (mountEl) {
        mountEl.style.backgroundColor = bgCol;
        applyTerminalBackgroundToMount(mountEl);
      }

      // 接管并注入精准闪烁节奏与颜色变量
      applyCursorBlinkToTerminal(
        term,
        terminalSettings.value.cursor_blink,
        terminalSettings.value.blink_interval,
        fgCol,
        bgCol,
        terminalSettings.value.cursor_style
      );

      s.fitInstance?.fit();
    }
  });

  if (!configData.value.settings) configData.value.settings = {};
  configData.value.settings.app_font_family = appFontFamily.value;
  configData.value.settings.font_family = terminalSettings.value.font_family;
  configData.value.settings.font_size = terminalSettings.value.font_size;
  configData.value.settings.is_bold = terminalSettings.value.is_bold;
  configData.value.settings.cursor_style = terminalSettings.value.cursor_style;
  configData.value.settings.cursor_blink = terminalSettings.value.cursor_blink;
  configData.value.settings.blink_interval = terminalSettings.value.blink_interval;
  configData.value.settings.terminal_bg_mode = terminalSettings.value.term_bg_mode;
  configData.value.settings.terminal_fg_color = fgCol;
  configData.value.settings.terminal_bg_color = bgCol;
  configData.value.settings.custom_bg_image = terminalSettings.value.custom_bg_image;
  configData.value.settings.terminal_bg_image = terminalSettings.value.custom_bg_image;
  configData.value.settings.terminal_bg_image_fit = terminalSettings.value.bg_image_fit;
  configData.value.settings.terminal_bg_image_dim = terminalSettings.value.bg_image_dim;
  configData.value.settings.terminal_bg_image_auto_fg = terminalSettings.value.bg_image_auto_fg;
  configData.value.settings.minimize_to_tray = terminalSettings.value.minimize_to_tray;
  configData.value.settings.accent_color = terminalSettings.value.accent_color;
  applyAccentColor(terminalSettings.value.accent_color);
  invoke("save_hosts_config", { config: configData.value });
  if (shouldClose) {
    showTerminalSettingsModal.value = false;
  }
}

// --- 会话连接与多会话 ---
async function openSession(h: HostItem) {
  if (currentView.value !== "workspace") {
    currentView.value = "workspace";
    await nextTick();
  }

  const sid = `sess-${Date.now()}`;
  const newTab: SessionTab = shallowReactive({
    id: sid,
    name: h.name,
    host: h,
    alive: false,
    is_connecting: true,
  });
  sessions.value.push(newTab);
  activeSessionId.value = sid;
  clearSftpState();
  resetNetworkHistory();
  await nextTick();

  const mountEl = document.getElementById(`term-${sid}`);
  if (!mountEl) {
    // 不能静默 return，否则会话会永久停在“连接中”状态
    newTab.is_connecting = false;
    newTab.alive = false;
    console.error("[session] 终端挂载点不存在:", sid);
    return;
  }

  const isDark = terminalSettings.value.term_bg_mode === "dark";
  const bgCol = isDark ? "#0A0B0E" : "#F4F5F7";
  const hasBgImage = !!terminalBgImageData.value;
  const fgCol = hasBgImage
    ? terminalSettings.value.terminal_fg_color || (isDark ? "#81C995" : "#111318")
    : ensureReadableFg(
        terminalSettings.value.terminal_fg_color,
        bgCol,
        isDark ? "#81C995" : "#111318",
      );

  mountEl.style.backgroundColor = bgCol;
  applyTerminalBackgroundToMount(mountEl);

  const term = new Terminal({
    fontFamily: terminalSettings.value.font_family,
    fontSize: terminalSettings.value.font_size,
    fontWeight: terminalSettings.value.is_bold ? "bold" : "normal",
    lineHeight: 1.25,
    cursorBlink: terminalSettings.value.cursor_blink,
    cursorStyle: terminalSettings.value.cursor_style,
    cursorInactiveStyle: terminalSettings.value.cursor_style,
    allowTransparency: true,
    theme: {
      background: hasBgImage ? "rgba(0,0,0,0)" : bgCol,
      foreground: fgCol,
      cursor: fgCol,
      cursorAccent: bgCol,
      selectionBackground: isDark ? "rgba(168, 199, 250, 0.3)" : "#CBD5E1",
      black: isDark ? "#1A1D24" : "#000000",
      red: "#F28B82",
      green: "#81C995",
      yellow: "#FDD663",
      blue: isDark ? "#78D9EC" : "#1A73E8",
      magenta: "#D7AEFB",
      cyan: "#78D9EC",
      white: isDark ? "#FFFFFF" : "#1F2937",
    }
  });

  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(mountEl);
  fit.fit();
  term.focus();
  newTab.termInstance = term;
  newTab.fitInstance = fit;

  // 1:1 对标 ssh8.py: 接管并注入精准闪烁节奏与颜色变量
  applyCursorBlinkToTerminal(
    term,
    terminalSettings.value.cursor_blink,
    terminalSettings.value.blink_interval,
    fgCol,
    bgCol,
    terminalSettings.value.cursor_style
  );
  // Ensure terminal receives focus so xterm adds .xterm-cursor-blink class
  term.focus();

  // 全量键盘快捷键捕获 (复制/粘贴/全选)
    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
    if (e.type !== "keydown") return true;
    const key = String(e.key || "").toLowerCase();
    if (newTab.read_only) {
      // 只读模式只允许复制/选择，不允许粘贴或发送输入
      if ((e.ctrlKey && e.shiftKey && key === "v") || (e.shiftKey && key === "insert")) {
        e.preventDefault();
        e.stopPropagation();
        return false;
      }
    }
    // 复制快捷键：Ctrl+Shift+C 或 Ctrl+Insert
    if ((e.ctrlKey && e.shiftKey && key === "c") || (e.ctrlKey && key === "insert")) {
      const sel = term.getSelection();
      e.preventDefault();
      e.stopPropagation();
      if (sel) void writeClipboardText(sel);
      return false;
    }
    // 粘贴快捷键：Ctrl+Shift+V 或 Shift+Insert
    if ((e.ctrlKey && e.shiftKey && key === "v") || (e.shiftKey && key === "insert")) {
      // 阻止 WebView2 默认粘贴，否则会和下面的自定义写入各执行一次。
      e.preventDefault();
      e.stopPropagation();
      void readClipboardText().then(async (text) => {
        if (!text || !newTab.alive) return;
        if (isGenericProtocol(h.protocol)) {
          await invoke("write_generic_session", { sessionId: sid, data: text });
        } else {
          await invoke("write_ssh_session", { sessionId: sid, data: text });
        }
        term.focus();
      });
      return false;
    }
    return true;
  });

  term.writeln(`\x1b[36m[QShell]\x1b[0m 正在连接到 ${h.name} (${h.host}:${h.port})...`);

  if (isTauri) {
    // 终端输入监听：1:1 对标 ssh8.py send_to_ssh
    // 正常连接时转发到 SSH；断开状态下按回车触发 reconnectSession 重连
    term.onData((data) => {
      if (newTab.read_only) return;
      if (newTab.alive) {
        if (isGenericProtocol(h.protocol)) {
          invoke("write_generic_session", { sessionId: sid, data });
        } else {
          invoke("write_ssh_session", { sessionId: sid, data });
        }
      } else {
        if (data.includes("\r") || data.includes("\n")) {
          reconnectSession(newTab);
        }
      }
    });

    const unlisten = await listen<string>(`ssh-data-${sid}`, (event) => {
      term.write(event.payload);
    });
    newTab.unlisten = unlisten;

    // 监听断开事件：1:1 对标 ssh8.py handle_terminal_disconnected
    listen(`ssh-closed-${sid}`, () => {
      newTab.alive = false;
      newTab.is_connecting = false;
      newTab.latency = 0;
      sessionMonitorStats.delete(sid);

      // 断开后必须立刻冻结该会话的监控和 SFTP 视图，避免继续展示远端状态。
      if (activeSessionId.value === sid) {
        stopMonitoring();
        resetMonitorStats();
        clearSftpState();
        isSftpDragOver.value = false;
      }

      term.writeln("\r\n\x1b[33m[终端连接已断开。按回车键重新连接...]\x1b[0m\r\n");
    });

    await doStartSsh(newTab);
  } else {
    // 浏览器预览模式 (兼容 http://localhost:1420 离线/浏览器端即时体验)
    newTab.alive = true;
    term.writeln(`\x1b[32m[QShell]\x1b[0m 连接成功 (浏览器预览模式: root@${h.host})`);
    term.write(`root@${h.host}:~# `);
    let curCmd = "";
    term.onData((data) => {
      if (data === "\r") {
        term.write("\r\n");
        if (curCmd.trim() === "clear") {
          term.clear();
        } else if (curCmd.trim()) {
          term.writeln(`\x1b[90m执行命令:\x1b[0m ${curCmd}`);
        }
        curCmd = "";
        term.write(`root@${h.host}:~# `);
      } else if (data === "\x7f") {
        if (curCmd.length > 0) {
          curCmd = curCmd.slice(0, -1);
          term.write("\b \b");
        }
      } else {
        curCmd += data;
        term.write(data);
      }
    });

    let resizeTimer: number | null = null;
    const resizeObserver = new ResizeObserver(() => {
      if (term.element) {
        if (resizeTimer) clearTimeout(resizeTimer);
        resizeTimer = window.setTimeout(() => {
          try { fit.fit(); } catch (_) {}
        }, 16);
      }
    });
    resizeObserver.observe(mountEl);
    newTab.resizeObserver = resizeObserver;
  }
}

// 统一的终端会话启动逻辑 (支持 SSH / Telnet / Serial 初次连接与重连)
async function doStartSsh(tab: SessionTab) {
  const sid = tab.id;
  const h = tab.host;
  const term = tab.termInstance;
  const fit = tab.fitInstance;
  if (!term) return;

  let sessionPassword = h.pwd || tab.runtimePassword || null;
  if (isTauri && !isGenericProtocol(h.protocol) && !h.pwd?.trim() && !h.key_path && !tab.runtimePassword) {
    const result = await askSshPassword(tab);
    if (!result) {
      tab.is_connecting = false;
      tab.alive = false;
      term.writeln("\r\n\x1b[33m[已取消连接]\x1b[0m\r\n");
      return;
    }
    sessionPassword = result.password;
    if (result.remember) {
      h.pwd = result.password;
      const managed = configData.value.managed_hosts?.find(
        item => item.host === h.host && item.port === h.port
      );
      if (managed) managed.pwd = result.password;
      try {
        await invoke("save_hosts_config", { config: configData.value });
      } catch (error) {
        console.warn("[ssh] 保存记住的密码失败:", error);
      }
    } else {
      tab.runtimePassword = result.password;
    }
  }

  try {
    if (h.protocol === "local") {
      await invoke<string>("start_local_shell_session", {
        sessionId: sid,
        shell: h.local_shell || "powershell",
        cols: term.cols || 80,
        rows: term.rows || 24,
        cwd: null,
      });
    } else if (h.protocol === "telnet") {
      await invoke<string>("start_telnet_session", {
        sessionId: sid,
        host: h.host,
        port: h.port || 23,
        cols: term.cols || 80,
        rows: term.rows || 24,
        termType: h.term_type || "auto",
      });
    } else if (h.protocol === "serial") {
      await invoke<string>("start_serial_session", {
        sessionId: sid,
        portName: h.host || "COM1",
        baudRate: h.port || 9600,
        options: {
          dataBits: h.serial_data_bits ?? 8,
          parity: h.serial_parity ?? "none",
          stopBits: h.serial_stop_bits ?? 1,
          flowControl: h.serial_flow_control ?? "none",
          dtr: h.serial_dtr ?? true,
          rts: h.serial_rts ?? true,
          newline: h.serial_newline ?? "cr",
          localEcho: h.serial_local_echo ?? false,
        }
      });
    } else {
      await invoke<string>("start_ssh_session", {
        params: {
          session_id: sid,
          host: h.host,
          port: h.port,
          username: h.user,
          password: sessionPassword,
          private_key: h.key_path || null,
          passphrase: h.passphrase || null,
          cols: term.cols || 80,
          rows: term.rows || 24,
          use_agent: !!h.use_agent,
          term_type: h.term_type || "auto",
        }
      });
    }
    tab.alive = true;
    tab.is_connecting = false;

    // 监听视口尺寸变化 (防抖过滤高频重排与 IPC 通信，大幅提升窗口缩放帧率)
    if (!tab.resizeObserver && fit && (term as any).element?.parentElement) {
      const mountEl = (term as any).element.parentElement;
      let resizeTimer: number | null = null;
      const resizeObserver = new ResizeObserver(() => {
        if (tab.alive && term.element) {
          if (resizeTimer) clearTimeout(resizeTimer);
          resizeTimer = window.setTimeout(() => {
            try {
              fit.fit();
              sendTerminalResize(tab, term.cols, term.rows);
            } catch (_) {}
          }, 32);
        }
      });
      resizeObserver.observe(mountEl);
      tab.resizeObserver = resizeObserver;
    }

    // 终端自带 onResize 监听兜底
    term.onResize(({ cols, rows }) => {
      sendTerminalResize(tab, cols, rows);
    });

    // 仅 SSH 模式启动服务器资源监控
    if (!isGenericProtocol(h.protocol)) {
      startMonitoringLoop(sid);
    }
  } catch (err: any) {
    tab.alive = false;
    tab.is_connecting = false;
    term.writeln(`\x1b[31m[连接失败]\x1b[0m ${err}\r\n`);
    term.writeln("\x1b[33m[按回车键重新尝试连接...]\x1b[0m\r\n");
    resetMonitorStats();
  }
}

// 1:1 对标 ssh8.py reconnect_session: 在终端中断或连接失败后，用户按回车发起重连
async function reconnectSession(tab: SessionTab) {
  if (tab.alive || tab.is_connecting) return;
  const term = tab.termInstance;
  if (!term) return;

  tab.is_connecting = true;
  tab.alive = false;
  resetMonitorStats();

  // 1:1 对标 ssh8.py 提示文本
  term.writeln("\r\n\x1b[36m[正在尝试重新连接终端...]\x1b[0m");

  if (isTauri) {
    // 先关闭远端旧可能存在的连接资源
    try {
      if (isGenericProtocol(tab.host?.protocol)) {
        await invoke("close_generic_session", { sessionId: tab.id });
      } else {
        await invoke("close_ssh_session", { sessionId: tab.id });
      }
    } catch (_) {}

    await doStartSsh(tab);
  } else {
    // 浏览器预览模式重连
    setTimeout(() => {
      tab.alive = true;
      tab.is_connecting = false;
      term.writeln(`\x1b[32m[QShell]\x1b[0m 重连成功 (浏览器预览模式: root@${tab.host.host})`);
      term.write(`root@${tab.host.host}:~# `);
    }, 400);
  }
}

let latencyInterval: number | null = null;

// 独立即时测速函数：连上或切到终端时毫秒级触发，不被任何耗时的系统指标采集阻塞
async function probeSessionLatency(sid: string) {
  if (activeSessionId.value !== sid) return;
  const currentTab = sessions.value.find(s => s.id === sid);
  if (!currentTab?.alive || !currentTab?.host?.host) return;
  try {
    const latRes = await invoke<string>("probe_latency", {
      host: currentTab.host.host.trim(),
      port: currentTab.host.port || 22,
      mode: "icmp",
    });
    const m = latRes.match(/([0-9]+(?:\.[0-9]+)?)\s*ms/);
    let probedVal = 0;
    if (m) {
      probedVal = Math.round(parseFloat(m[1]));
    } else if (latRes.includes("<1")) {
      probedVal = 1;
    }
    if (probedVal > 0) {
      const liveTab = sessions.value.find(s => s.id === sid);
      if (!liveTab?.alive) return;
      currentTab.latency = probedVal; // 会话级持久化缓存，切换标签秒出
      if (activeSessionId.value === sid) {
        monitorStats.value.latency = probedVal;
      }
    }
  } catch (_) {}
}

function stopMonitoring() {
  if (monitorInterval) {
    clearInterval(monitorInterval);
    monitorInterval = null;
  }
  if (latencyInterval) {
    clearInterval(latencyInterval);
    latencyInterval = null;
  }
  refreshMetricsNow = null;
}

function startMonitoringLoop(sid: string) {
  if (monitorInterval) { clearInterval(monitorInterval); monitorInterval = null; }
  if (latencyInterval) { clearInterval(latencyInterval); latencyInterval = null; }

  const tab = sessions.value.find(s => s.id === sid);
  if (!tab || isGenericProtocol(tab.host?.protocol)) {
    // Telnet / Serial / 本地终端不支持 SSH SFTP 与硬件探针，避免触发异常
    return;
  }

  // 1. 终端连上后【立刻】执行首次测速
  probeSessionLatency(sid);
  // 保持 3 秒一次心跳测速
  latencyInterval = window.setInterval(() => {
    probeSessionLatency(sid);
  }, 3000);

  // 2. 独立的系统指标采集轮询 (3秒心跳)
  const pollMetrics = async () => {
    if (activeSessionId.value !== sid) return;
    const liveTab = sessions.value.find(s => s.id === sid);
    if (!liveTab?.alive) {
      stopMonitoring();
      return;
    }
    // 上一轮还没返回就跳过本轮，避免慢速/挂起的采集把待处理任务越堆越多。
    if (metricsPollInFlight) return;
    metricsPollInFlight = true;
    try {
      const stats = await invoke<{
        cpu: number;
        mem: number;
        disk: number;
        net_up: string;
        net_down: string;
        net_if?: string;
        net_if_options?: string[];
        cpu_model?: string;
        cpu_cores?: number;
        mem_total_bytes?: number;
        mem_used_bytes?: number;
        disk_total_bytes?: number;
        disk_used_bytes?: number;
      }>(
        "fetch_system_metrics", 
        {
          sessionId: sid,
          interface: activeSession.value?.net_if || null,
        }
      );
      if (stats && activeSessionId.value === sid && sessions.value.find(s => s.id === sid)?.alive) {
        const next = {
          cpu: stats.cpu,
          mem: stats.mem,
          disk: stats.disk,
          net_up: stats.net_up || "0 KB/s",
          net_down: stats.net_down || "0 KB/s",
          net_if: stats.net_if || "",
          net_if_options: stats.net_if_options || [],
          latency: monitorStats.value.latency,
          cpu_model: stats.cpu_model || "",
          cpu_cores: stats.cpu_cores || 0,
          mem_total_bytes: stats.mem_total_bytes || 0,
          mem_used_bytes: stats.mem_used_bytes || 0,
          disk_total_bytes: stats.disk_total_bytes || 0,
          disk_used_bytes: stats.disk_used_bytes || 0,
        };
        sessionMonitorStats.set(sid, next);
        monitorStats.value = next;
        recordNetworkSample(next.net_up, next.net_down);
      }
    } catch (e) {
      console.warn("系统指标采集返回 (保留上一次快照):", e);
    } finally {
      metricsPollInFlight = false;
    }
  };
  refreshMetricsNow = pollMetrics;
  pollMetrics();
  monitorInterval = window.setInterval(pollMetrics, 3000);
}

function switchSession(sid: string) {
  if (activeSessionId.value === sid) return;
  activeSessionId.value = sid;
  clearSftpState();
  // 切标签时恢复该会话自己的监控快照，避免串到上一台主机
  resetNetworkHistory();
  const cached = sessionMonitorStats.get(sid);
  const cur = sessions.value.find(s => s.id === sid);
  const cachedLatency = cur?.latency && cur.latency > 0 ? cur.latency : 0;
  monitorStats.value = cached
    ? { ...cached, latency: cachedLatency || cached.latency }
    : {
        cpu: 0,
        mem: 0,
        disk: 0,
        net_up: "0 KB/s",
        net_down: "0 KB/s",
        net_if: "",
        net_if_options: [],
        latency: cachedLatency,
        cpu_model: "",
        cpu_cores: 0,
        mem_total_bytes: 0,
        mem_used_bytes: 0,
        disk_total_bytes: 0,
        disk_used_bytes: 0,
      };
  startMonitoringLoop(sid);

  // 解决切换标签页吞字/重叠：等 DOM 切换显示并完成几何计算后执行多帧适配与刷新
  const doRefitAndRefresh = () => {
    if (activeSessionId.value !== sid) return;
    if (cur?.termInstance && cur.fitInstance) {
      try {
        cur.fitInstance.fit();
        sendTerminalResize(cur, cur.termInstance.cols, cur.termInstance.rows);
        cur.termInstance.refresh(0, cur.termInstance.rows - 1);
        cur.termInstance.focus();
      } catch (_) {}
    }
  };

  nextTick(() => {
    doRefitAndRefresh();
    requestAnimationFrame(() => {
      doRefitAndRefresh();
      setTimeout(doRefitAndRefresh, 60);
    });
  });
}

function closeSession(tab: SessionTab) {
  if (tab.is_locked) return; // 锁定状态禁止关闭
  if (tab.resizeObserver) tab.resizeObserver.disconnect();
  if (tab.unlisten) tab.unlisten();
  if (tab.termInstance) tab.termInstance.dispose();
  if (isGenericProtocol(tab.host?.protocol)) {
    invoke("close_generic_session", { sessionId: tab.id });
  } else {
    invoke("close_ssh_session", { sessionId: tab.id });
  }
  sessionMonitorStats.delete(tab.id);
  const wasActive = activeSessionId.value === tab.id;
  sessions.value = sessions.value.filter(s => s.id !== tab.id);
  if (sessions.value.length === 0) {
    activeSessionId.value = "";
    clearSftpState();
    if (monitorInterval) clearInterval(monitorInterval);
    resetMonitorStats();
  } else if (wasActive) {
    const nextTab = sessions.value[sessions.value.length - 1];
    activeSessionId.value = nextTab.id;
    clearSftpState();
    startMonitoringLoop(nextTab.id);
  }
}

const sftpErrorMsg = ref("");

const activeSessionIsSsh = computed(() => {
  const sess = activeSession.value;
  if (!sess) return false;
  // 只有 SSH 协议支持 SFTP/监控；本地终端、Telnet、串口都不算
  return sess.host?.protocol === "ssh" || !sess.host?.protocol;
});

// 严格判断当前前台活跃会话是否支持 SFTP 文件传输能力 (仅限 SSH 协议且会话在线)
const canUseSftp = computed(() => {
  const sess = activeSession.value;
  return !!sess?.alive && activeSessionIsSsh.value;
});

// 面板与右键菜单共用同一套状态，避免把"无会话/连接中/已断开"误标成"协议不支持"。
const sftpPanelState = computed<"no-session" | "non-ssh" | "connecting" | "disconnected" | "ready">(() => {
  const sess = activeSession.value;
  if (!sess) return "no-session";
  if (!activeSessionIsSsh.value) return "non-ssh";
  if (sess.alive) return "ready";
  return sess.is_connecting ? "connecting" : "disconnected";
});

const sftpPanelStateText = computed(() => {
  switch (sftpPanelState.value) {
    case "no-session": return "请先连接 SSH 主机";
    case "non-ssh": return "当前协议不支持 SFTP";
    case "connecting": return "SSH 会话连接中...";
    case "disconnected": return "SSH 会话已断开";
    default: return "";
  }
});

function clearSftpState() {
  sftpRequestSeq++;
  sftpFiles.value = [];
  selectedSftpFiles.value = [];
  selectedSftpFile.value = null;
  sftpSelectionAnchorIndex = -1;
  currentSftpPath.value = "/";
  sftpErrorMsg.value = "";
  isSftpLoading.value = false;
  isSftpDragOver.value = false;
}

function switchRightPanelTab(tab: "sftp" | "memo") {
  rightPanelTab.value = tab;
  if (tab === "sftp" && activeSessionId.value) {
    fetchSftpHome();
  }
}

// --- SFTP 核心逻辑 ---
async function fetchSftpHome() {
  const sessionId = activeSessionId.value;
  if (!isTauri || !sessionId || !canUseSftp.value) return;

  const requestId = ++sftpRequestSeq;
  isSftpLoading.value = true;
  sftpErrorMsg.value = "";
  try {
    const home = await invoke<string>("sftp_home_dir", { sessionId });
    if (requestId !== sftpRequestSeq || activeSessionId.value !== sessionId) return;
    await fetchSftpFiles(home || "/", sessionId);

    // A newly-opened SSH session can expose its SFTP subsystem a fraction
    // later than the shell. If the first listing is empty, retry once after
    // the session settles instead of leaving the misleading empty state.
    if (sftpFiles.value.length === 0 && requestId === sftpRequestSeq && activeSessionId.value === sessionId && canUseSftp.value) {
      isSftpLoading.value = true;
      await new Promise(resolve => window.setTimeout(resolve, 260));
      // 等待期间用户可能已手动跳转目录或切换会话，必须重新确认这次重试仍然有效，
      // 否则 260ms 后旧重试会把列表和路径覆盖回 home。
      if (requestId === sftpRequestSeq && activeSessionId.value === sessionId && canUseSftp.value) {
        await fetchSftpFiles(home || "/", sessionId);
      } else if (requestId === sftpRequestSeq) {
        isSftpLoading.value = false;
      }
    }
  } catch {
    if (requestId !== sftpRequestSeq || activeSessionId.value !== sessionId) return;
    await fetchSftpFiles("/", sessionId);
  }
}

function sortSftpList(list: SftpFileItem[]): SftpFileItem[] {
  const key = sftpSortKey.value;
  const direction = sftpSortAsc.value ? 1 : -1;
  return [...list].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    let result = 0;
    if (key === "size") {
      result = (a.size_bytes || 0) - (b.size_bytes || 0);
    }
    if (result === 0) {
      result = a.name.localeCompare(b.name, "zh-CN", {
        numeric: true,
        sensitivity: "base",
      });
    }
    return result * direction;
  });
}

function applySftpSort() {
  sftpFiles.value = sortSftpList(sftpFiles.value);
}

function toggleSftpSort(key: "name" | "size") {
  if (sftpSortKey.value === key) {
    sftpSortAsc.value = !sftpSortAsc.value;
  } else {
    sftpSortKey.value = key;
    // 名称默认升序；大小首次点击按从大到小更符合直觉
    sftpSortAsc.value = key === "name";
  }
  applySftpSort();
}

async function fetchSftpFiles(path: string, expectedSessionId = activeSessionId.value) {
  const sessionId = expectedSessionId;
  if (!isTauri || !sessionId || activeSessionId.value !== sessionId || !canUseSftp.value) return;
  const requestId = ++sftpRequestSeq;
  const stillCurrent = () =>
    requestId === sftpRequestSeq &&
    activeSessionId.value === sessionId &&
    !!sessions.value.find(s => s.id === sessionId)?.alive;
  isSftpLoading.value = true;
  sftpErrorMsg.value = "";
  try {
    const list = await invoke<SftpFileItem[]>("list_sftp_files", {
      sessionId,
      remotePath: path,
    });
    if (!stillCurrent()) return;
    currentSftpPath.value = path;
    sftpFiles.value = sortSftpList(list);
    selectedSftpFiles.value = [];
    selectedSftpFile.value = null;
    sftpSelectionAnchorIndex = -1;
  } catch (err: any) {
    if (!stillCurrent()) return;
    console.error("读取 SFTP 目录失败:", err);
    // 读取失败时清空旧列表，避免错误提示与上一个目录的内容同时显示
    sftpFiles.value = [];
    selectedSftpFiles.value = [];
    selectedSftpFile.value = null;
    sftpSelectionAnchorIndex = -1;
    sftpErrorMsg.value = `SFTP 读取失败: ${err}`;
  } finally {
    if (requestId === sftpRequestSeq) {
      isSftpLoading.value = false;
    }
  }
}

// SFTP 跟随当前活动 SSH 会话：会话切换、从连接中变为在线、重连成功后
// 都在这里统一刷新，避免切换标签时只看一次旧状态。
watch(
  [activeSessionId, () => activeSession.value?.alive, isRightPanelOpen, rightPanelTab],
  () => {
    // 右栏折叠时切换会话不会触发刷新，展开后必须补一次；切到 SFTP 页签同理。
    if (!isRightPanelOpen.value || rightPanelTab.value !== "sftp") return;
    if (!activeSessionId.value || !activeSession.value?.alive) return;
    if (!canUseSftp.value) return;
    clearSftpState();
    void fetchSftpHome();
  },
);

function sftpNavigateParent() {
  if (currentSftpPath.value === "/" || !currentSftpPath.value) return;
  const parts = currentSftpPath.value.split("/").filter(Boolean);
  parts.pop();
  const parent = "/" + parts.join("/");
  fetchSftpFiles(parent || "/");
}

function isSftpSelected(item: SftpFileItem): boolean {
  return selectedSftpFiles.value.some((f) => f.name === item.name);
}

function handleSftpItemKeydown(event: KeyboardEvent, item: SftpFileItem, index: number) {
  if (event.key === "Enter") {
    event.preventDefault();
    // 目录进入下一层；文件直接触发下载
    if (item.is_dir) {
      sftpCd(item);
    } else {
      void sftpTriggerDownload(item);
    }
    return;
  }
  if (event.key === " " || event.key === "Spacebar") {
    event.preventDefault();
    handleSftpItemClick(item, index, event as unknown as MouseEvent);
    return;
  }
  // Shift+F10 / 菜单键：键盘用户也能打开右键菜单
  if ((event.key === "F10" && event.shiftKey) || event.key === "ContextMenu") {
    event.preventDefault();
    const el = event.currentTarget as HTMLElement | null;
    const rect = el?.getBoundingClientRect();
    const pseudo = {
      clientX: rect ? rect.left + 24 : 0,
      clientY: rect ? rect.top + 16 : 0,
      preventDefault() {},
    } as unknown as MouseEvent;
    handleSftpContextMenu(pseudo, item, index);
  }
}

function handleSftpItemClick(item: SftpFileItem, index: number, event: MouseEvent) {
  const additive = event.ctrlKey || event.metaKey;
  const range = event.shiftKey;
  if (range) {
    if (sftpSelectionAnchorIndex < 0) sftpSelectionAnchorIndex = index;
    const start = Math.min(sftpSelectionAnchorIndex, index);
    const end = Math.max(sftpSelectionAnchorIndex, index);
    selectedSftpFiles.value = sftpFiles.value.slice(start, end + 1);
    selectedSftpFile.value = item;
    return;
  }
  if (additive) {
    if (isSftpSelected(item)) {
      selectedSftpFiles.value = selectedSftpFiles.value.filter((f) => f.name !== item.name);
      selectedSftpFile.value = selectedSftpFiles.value[selectedSftpFiles.value.length - 1] || null;
    } else {
      selectedSftpFiles.value = [...selectedSftpFiles.value, item];
      selectedSftpFile.value = item;
    }
    sftpSelectionAnchorIndex = index;
    return;
  }
  selectedSftpFiles.value = [item];
  selectedSftpFile.value = item;
  sftpSelectionAnchorIndex = index;
}

function handleSftpContextMenu(event: MouseEvent, item: SftpFileItem, index: number) {
  if (!isSftpSelected(item)) {
    selectedSftpFiles.value = [item];
    selectedSftpFile.value = item;
    sftpSelectionAnchorIndex = index;
  }
  openContextMenu(event, "sftp_item", undefined, undefined, item);
}

async function sftpTriggerDownloadSelected() {
  const files = selectedSftpFiles.value.filter((f) => !f.is_dir);
  if (!files.length) return;
  // 只能串行：openLocalFileBrowser 是单例，并发调用会互相取消，导致只有最后一个文件真正下载。
  for (const file of files) {
    const saved = await sftpTriggerDownload(file);
    if (!saved) break;
  }
}

function sftpCd(item: SftpFileItem) {
  if (!item.is_dir) return;
  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  fetchSftpFiles(base + item.name);
}

const isSftpDragOver = ref(false);

async function remotePathExists(remotePath: string): Promise<boolean> {
  if (!activeSessionId.value) return false;
  try {
    await invoke("sftp_stat", { sessionId: activeSessionId.value, remotePath });
    return true;
  } catch {
    return false;
  }
}

async function localPathExists(path: string): Promise<boolean> {
  try {
    return await invoke<boolean>("local_path_exists", { path });
  } catch {
    return false;
  }
}

function splitFileName(name: string): { base: string; ext: string } {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return { base: name, ext: "" };
  return { base: name.slice(0, dot), ext: name.slice(dot) };
}

async function availableRemoteName(name: string, dir: string): Promise<string> {
  const taken = new Set(sftpFiles.value.map(f => f.name));
  if (!taken.has(name) && !(await remotePathExists(dir + name))) return name;
  const { base: stem, ext } = splitFileName(name);
  for (let i = 1; i < 1000; i++) {
    const candidate = `${stem} (${i})${ext}`;
    if (taken.has(candidate)) continue;
    // 内存里的 sftpFiles 可能已过期（外部新建/读取失败），候选名必须再向远端确认一次，
    // 否则会把同一个名字传给后端，rename 时静默覆盖已有文件。
    if (!(await remotePathExists(dir + candidate))) return candidate;
  }
  return `${stem} (${Date.now()})${ext}`;
}

async function availableLocalPath(path: string): Promise<string> {
  if (!(await localPathExists(path))) return path;
  const dir = path.replace(/[\\/][^\\/]*$/, "");
  const current = path.split(/[/\\]/).pop() || "download";
  const { base, ext } = splitFileName(current);
  for (let i = 1; i < 1000; i++) {
    const candidate = `${dir}\\${base} (${i})${ext}`;
    if (!(await localPathExists(candidate))) return candidate;
  }
  return `${dir}\\${base} (${Date.now()})${ext}`;
}

async function uploadLocalPath(localPath: string) {
  if (!canUseSftp.value || !activeSessionId.value) return;
  let fname = localPath.split(/[/\\]/).pop() || "upload_file";
  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  let remotePath = base + fname;

  let overwrite = false;
  if (await remotePathExists(remotePath)) {
    const choice = await askTransferConflict(
      "远端文件已存在",
      `当前目录已存在“${fname}”，请选择处理方式。`,
      fname,
    );
    if (choice === "cancel") return;
    if (choice === "skip") return;
    if (choice === "rename") {
      fname = await availableRemoteName(fname, base);
      remotePath = base + fname;
    } else {
      // 只有用户明确选了"覆盖"才允许后端覆盖，其余情况后端会拒绝写入。
      overwrite = true;
    }
  }

  const taskId = makeTaskId("up");

  showTransferMonitor.value = true;
  activeTransfers.value.push({
    task_id: taskId,
    filename: fname,
    is_upload: true,
    transferred: 0,
    total: 100,
    speed_bps: 0,
    status: "running",
  });

  try {
    await invoke("sftp_upload_file", {
      sessionId: activeSessionId.value,
      localPath,
      remotePath,
      taskId,
      overwrite,
    });
    fetchSftpFiles(currentSftpPath.value);
  } catch (e: any) {
    console.error("上传错误:", e);
    const task = activeTransfers.value.find(t => t.task_id === taskId);
    if (task) {
      const raw = String(e?.message || e || "上传失败");
      task.status = "failed";
      task.speed_bps = 0;
      task.error = /permission denied|permission|权限/i.test(raw)
        ? "目标目录没有写入权限，请切换到用户 home 或其他可写目录"
        : raw;
    }
  }
}

// SFTP 文件上传 (按钮触发，使用应用内文件选择器)
async function sftpTriggerUpload() {
  closeContextMenu();
  if (!activeSessionId.value) return;
  const path = await openLocalFileBrowser({
    action: "open",
    title: "选择要上传的文件",
  });
  if (path) uploadLocalPath(path);
}

function finishLocalFileBrowser(path: string | null) {
  const resolve = localPickerResolve;
  localPickerResolve = null;
  showLocalFilePicker.value = false;
  if (resolve) resolve(path);
}

function cancelLocalFileBrowser() {
  finishLocalFileBrowser(null);
}

// 只允许"单个文件名"：去掉任何路径分隔符与非法字符，防止 ../ 或绝对路径逃逸出所选目录。
const WINDOWS_RESERVED_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;
function sanitizeLocalFileName(raw: string): string {
  const lastSegment = String(raw || "").split(/[/\\]/).pop() || "";
  const stripped = lastSegment
    .replace(/[<>:"|?*\u0000-\u001f]/g, "")
    .replace(/[. ]+$/, "")
    .trim();
  if (!stripped || /^\.+$/.test(stripped)) return "";
  if (WINDOWS_RESERVED_NAME.test(stripped)) return "";
  return stripped;
}

function joinLocalPath(dir: string, name: string) {
  const cleanName = sanitizeLocalFileName(name);
  if (!cleanName) return "";
  return /[\\/]$/.test(dir) ? `${dir}${cleanName}` : `${dir}\\${cleanName}`;
}

async function openLocalFileBrowser(options: {
  action: "open" | "save";
  title: string;
  defaultName?: string;
}) {
  // 若之前还有未结束的选择器，先取消，避免 Promise 悬挂。
  if (localPickerResolve) finishLocalFileBrowser(null);

  localPickerAction.value = options.action;
  localPickerTitle.value = options.title;
  localPickerFileName.value = options.defaultName || "";
  localPickerSelected.value = null;
  localPickerError.value = "";
  showLocalFilePicker.value = true;

  const result = new Promise<string | null>((resolve) => {
    localPickerResolve = resolve;
  });

  try {
    if (localPickerRoots.value.length === 0) {
      localPickerRoots.value = await invoke<LocalFileEntry[]>("local_roots");
    }
    if (localKnownFolders.value.length === 0) {
      localKnownFolders.value = await invoke<Array<{ id: string; name: string; path: string; icon: string }>>("local_known_folders");
    }
    const saved = localStorage.getItem("qshell_last_local_dir") || "";
    const start = localPickerPath.value || saved || localPickerRoots.value[0]?.path || "C:\\";
    await loadLocalDir(start);
  } catch (e: any) {
    localPickerError.value = String(e?.message || e || "无法读取本地目录");
  }

  return result;
}

function isImageFileName(name: string): boolean {
  return /\.(png|jpe?g|webp|gif|bmp)$/i.test(name);
}

async function loadLocalPickerThumbnails(entries: LocalFileEntry[], token: number) {
  const pending = entries.filter(
    (entry) => !entry.is_dir && isImageFileName(entry.name) && !localPickerThumbnails.value[entry.path],
  );
  // 限制并发，避免大目录同时解码几十张图片导致文件浏览器卡顿。
  for (let offset = 0; offset < pending.length; offset += 4) {
    if (token !== localPickerDirToken) return;
    const batch = pending.slice(offset, offset + 4);
    await Promise.all(batch.map(async (entry) => {
      try {
        const data = await invoke<string>("local_image_thumbnail", {
          path: entry.path,
          maxSize: 48,
        });
        if (token === localPickerDirToken && data) {
          const cache = localPickerThumbnails.value;
          cache[entry.path] = data;
          // 缓存没有上限时，浏览大量图片目录后内存会单调增长；超出后按写入顺序淘汰。
          const keys = Object.keys(cache);
          if (keys.length > LOCAL_THUMB_CACHE_LIMIT) {
            for (const key of keys.slice(0, keys.length - LOCAL_THUMB_CACHE_LIMIT)) {
              delete cache[key];
            }
          }
        }
      } catch {
        // 损坏/不支持的文件保持普通文件图标。
      }
    }));
  }
}

async function loadLocalDir(path: string) {
  if (!path) return;
  const token = ++localPickerDirToken;
  localPickerLoading.value = true;
  localPickerError.value = "";
  try {
    const entries = await invoke<LocalFileEntry[]>("list_local_dir", { path });
    localPickerEntries.value = entries;
    localPickerPath.value = path;
    localPickerMode.value = "files";
    localPickerSelected.value = null;
    localStorage.setItem("qshell_last_local_dir", path);
    void loadLocalPickerThumbnails(entries, token);
  } catch (e: any) {
    localPickerError.value = String(e?.message || e || "读取目录失败");
  } finally {
    if (token === localPickerDirToken) {
      localPickerLoading.value = false;
    }
  }
}

function openLocalKnownFolder(folder: { path: string }) {
  localPickerSelected.value = null;
  loadLocalDir(folder.path);
}

function showLocalThisPc() {
  localPickerMode.value = "drives";
  localPickerPath.value = "此电脑";
  localPickerEntries.value = [];
  localPickerSelected.value = null;
  localPickerError.value = "";
}

function showLocalNetwork() {
  localPickerMode.value = "network";
  localPickerPath.value = "\\\\";
  localPickerEntries.value = [];
  localPickerSelected.value = null;
  localPickerError.value = "输入网络路径，例如 \\\\192.168.1.10\\share，然后按 Enter";
}

function localPickerParent() {
  if (localPickerMode.value !== "files") return;
  const path = localPickerPath.value.replace(/[\\/]+$/, "");
  if (!path || /^[A-Za-z]:$/.test(path)) return;
  const index = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  if (index < 0) return;
  const parent = path.slice(0, index);
  loadLocalDir(/^[A-Za-z]:$/.test(parent) ? parent + "\\" : parent || "/");
}

function localPickerChoose(entry: LocalFileEntry) {
  if (entry.is_dir) {
    loadLocalDir(entry.path);
  } else if (localPickerAction.value === "save") {
    localPickerFileName.value = entry.name;
  } else {
    localPickerSelected.value = entry;
  }
}

function confirmLocalFilePicker() {
  if (localPickerAction.value === "save") {
    if (localPickerMode.value !== "files") return;
    const target = joinLocalPath(localPickerPath.value, localPickerFileName.value);
    if (!target) {
      localPickerError.value = "文件名无效：不能包含 / \\ : * ? \" < > | ，也不能是 CON/NUL/COM1 等保留名";
      return;
    }
    finishLocalFileBrowser(target);
    return;
  }

  const selected = localPickerSelected.value;
  if (!selected || selected.is_dir) return;
  finishLocalFileBrowser(selected.path);
}

// SFTP 文件下载
async function sftpTriggerDownload(item: SftpFileItem): Promise<boolean> {
  closeContextMenu();
  if (!activeSessionId.value) return false;
  let targetSavePath = await openLocalFileBrowser({
    action: "save",
    title: `下载文件: ${item.name}`,
    defaultName: item.name,
  });
  if (!targetSavePath) return false;

  if (await localPathExists(targetSavePath)) {
    const choice = await askTransferConflict(
      "本地文件已存在",
      `所选位置已存在“${targetSavePath.split(/[/\\]/).pop()}”，请选择处理方式。`,
      targetSavePath.split(/[/\\]/).pop() || item.name,
    );
    if (choice === "cancel") return false;
    if (choice === "skip") return true;
    if (choice === "rename") {
      targetSavePath = await availableLocalPath(targetSavePath);
    }
  }

  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  const remotePath = base + item.name;
  const taskId = makeTaskId("down");
  const sessionId = activeSessionId.value;

  showTransferMonitor.value = true;
  activeTransfers.value.push({
    task_id: taskId,
    filename: item.name,
    is_upload: false,
    transferred: 0,
    total: item.size_bytes || 100,
    speed_bps: 0,
    status: "running",
  });

  try {
    await invoke("sftp_download_file", {
      sessionId,
      remotePath,
      localPath: targetSavePath,
      taskId,
    });
    return true;
  } catch (e: any) {
    console.error("下载错误:", e);
    const task = activeTransfers.value.find(t => t.task_id === taskId);
    if (task && task.status === "running") {
      const raw = String(e?.message || e || "下载失败");
      task.status = "failed";
      task.speed_bps = 0;
      task.error = /permission denied|permission|权限/i.test(raw)
        ? "目标位置没有写入权限，请选择其他保存目录"
        : raw;
    }
    return false;
  }
}

// SFTP 查看文件属性 (SFTPPropertiesDialog)
async function sftpViewProperties(item: SftpFileItem) {
  closeContextMenu();
  if (!activeSessionId.value) return;
  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  const fullPath = base + item.name;

  try {
    const props = await invoke<any>("sftp_stat", {
      sessionId: activeSessionId.value,
      remotePath: fullPath,
    });
    sftpPropsModal.value = {
      show: true,
      name: props.name,
      full_path: props.full_path,
      is_dir: props.is_dir,
      size_bytes: props.size_bytes,
      mode_str: props.mode_str,
      mode_oct: props.mode_oct,
      mtime_str: (() => {
        const ts = Number(props.mtime_str);
        return Number.isFinite(ts) && ts > 0 ? new Date(ts * 1000).toLocaleString() : "—";
      })(),
    };
  } catch (e) {
    console.error("获取文件属性失败:", e);
  }
}

// SFTP 修改权限 (SFTPChmodDialog)
function sftpOpenChmod(item: SftpFileItem) {
  closeContextMenu();
  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  const fullPath = base + item.name;

  let oct = parseInt(item.mode_oct || item.mode_str, 8) || 0o755;
  sftpChmodModal.value = {
    show: true,
    filePath: fullPath,
    fileName: item.name,
    u_r: !!(oct & 0o400), u_w: !!(oct & 0o200), u_x: !!(oct & 0o100),
    g_r: !!(oct & 0o040), g_w: !!(oct & 0o020), g_x: !!(oct & 0o010),
    o_r: !!(oct & 0o004), o_w: !!(oct & 0o002), o_x: !!(oct & 0o001),
  };
}

async function sftpSaveChmod() {
  const m = sftpChmodModal.value;
  let oct = 0;
  if (m.u_r) oct |= 0o400; if (m.u_w) oct |= 0o200; if (m.u_x) oct |= 0o100;
  if (m.g_r) oct |= 0o040; if (m.g_w) oct |= 0o020; if (m.g_x) oct |= 0o010;
  if (m.o_r) oct |= 0o004; if (m.o_w) oct |= 0o002; if (m.o_x) oct |= 0o001;

  const octStr = oct.toString(8).padStart(4, "0");
  try {
    await invoke("sftp_chmod", {
      sessionId: activeSessionId.value,
      remotePath: m.filePath,
      modeOct: octStr,
    });
    m.show = false;
    await fetchSftpFiles(currentSftpPath.value);
  } catch (e: any) {
    console.error("修改权限失败:", e);
    alert(`修改权限失败: ${e?.message || e}`);
  }
}

// SFTP 新建目录 / 重命名 / 删除
function sftpOpenNewFolder() {
  closeContextMenu();
  promptModal.value = {
    show: true,
    title: "新建远程文件夹",
    label: "文件夹名称:",
    value: "新建文件夹",
    action: "new_folder",
    payload: null,
  };
}

function sftpOpenRename(item: SftpFileItem) {
  closeContextMenu();
  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  promptModal.value = {
    show: true,
    title: `重命名: ${item.name}`,
    label: "新名称:",
    value: item.name,
    action: "rename_file",
    payload: { oldPath: base + item.name, base },
  };
}

async function executePromptModal() {
  const p = promptModal.value;
  const val = p.value.trim();
  if (!val) return;

  try {
    if (p.action === "new_device_group") {
      configData.value.devices.push({ group: val, hosts: [] });
      await invoke("save_hosts_config", { config: configData.value });
      p.show = false;
      return;
    } else if (p.action === "rename_device_group") {
      if (p.payload?.group) {
        p.payload.group.group = val;
        await invoke("save_hosts_config", { config: configData.value });
      }
      p.show = false;
      return;
    } else if (p.action === "new_memo_group") {
      if (!configData.value.memos) configData.value.memos = [];
      configData.value.memos.push({ group: val, items: [] });
      await invoke("save_hosts_config", { config: configData.value });
      p.show = false;
      return;
    } else if (p.action === "rename_memo_group") {
      const idx = p.payload?.groupIdx;
      if (idx !== undefined && configData.value.memos?.[idx]) {
        configData.value.memos[idx].group = val;
        await invoke("save_hosts_config", { config: configData.value });
      }
      p.show = false;
      return;
    }

    // 以下为 SFTP 文件操作 (依赖活动会话)
    if (!activeSessionId.value) return;

    if (p.action === "new_folder") {
      const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
      await invoke("sftp_mkdir", {
        sessionId: activeSessionId.value,
        remotePath: base + val,
      });
    } else if (p.action === "rename_file") {
      await invoke("sftp_rename", {
        sessionId: activeSessionId.value,
        oldPath: p.payload.oldPath,
        newPath: p.payload.base + val,
      });
    }
    p.show = false;
    fetchSftpFiles(currentSftpPath.value);
  } catch (e) {
    console.error("操作失败:", e);
  }
}

function sftpRefresh() {
  closeContextMenu();
  if (!canUseSftp.value || !activeSessionId.value) return;
  fetchSftpFiles(currentSftpPath.value);
}

async function sftpDeleteItem(item: SftpFileItem) {
  closeContextMenu();
  if (!activeSessionId.value) return;

  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  
  // 判断当前是否属于多选批量删除模式
  const isMulti = selectedSftpFiles.value.length > 1 && selectedSftpFiles.value.some(f => f.name === item.name);
  const targets = isMulti ? [...selectedSftpFiles.value] : [item];

  if (targets.length === 1) {
    const single = targets[0];
    const fullPath = base + single.name;
    customConfirmModal.value = {
      show: true,
      title: single.is_dir ? "删除文件夹" : "删除文件",
      message: `确定要删除${single.is_dir ? "文件夹" : "文件"} "${single.name}" 吗？\n\n${fullPath}\n\n此操作不可恢复。`,
      items: [],
      danger: true,
      confirmText: "确认删除",
      onConfirm: () => {
        customConfirmModal.value.show = false;
        void sftpDeleteItemsConfirmed(targets);
      },
    };
  } else {
    customConfirmModal.value = {
      show: true,
      title: `批量删除 (${targets.length} 项)`,
      message: `确定要删除选中的 ${targets.length} 个文件/文件夹吗？此操作不可恢复。\n\n已选待删除清单：`,
      items: targets.map(t => `${t.name}${t.is_dir ? " (文件夹)" : ""}`),
      danger: true,
      confirmText: `确认删除 (${targets.length})`,
      onConfirm: () => {
        customConfirmModal.value.show = false;
        void sftpDeleteItemsConfirmed(targets);
      },
    };
  }
}

async function sftpDeleteItemsConfirmed(targets: SftpFileItem[]) {
  if (!activeSessionId.value || !targets.length) return;
  // 循环期间用户可能切换标签，必须固定开始时解析出的会话与目录，
  // 否则后续删除会落到另一台主机的同名路径上。
  const sessionId = activeSessionId.value;
  const base = currentSftpPath.value.endsWith("/") ? currentSftpPath.value : currentSftpPath.value + "/";
  let failCount = 0;
  let lastError = "";

  for (const t of targets) {
    try {
      await invoke("sftp_remove", {
        sessionId,
        remotePath: base + t.name,
        isDir: t.is_dir,
      });
    } catch (e: any) {
      failCount++;
      lastError = e?.message || String(e);
      console.error(`删除 ${t.name} 失败:`, e);
    }
  }

  selectedSftpFiles.value = [];
  selectedSftpFile.value = null;
  await fetchSftpFiles(currentSftpPath.value);

  if (failCount > 0) {
    alert(`批量删除完成，其中 ${failCount} 项删除失败: ${lastError}`);
  }
}

function formatSpeed(bps: number) {
  if (bps < 1024) return `${bps} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(2)} MB/s`;
}

// --- 备忘录管理 ---
function sendMemoCommand(cmd: string) {
  const cur = sessions.value.find(s => s.id === activeSessionId.value);
  if (cur && cur.alive) {
    invoke("write_ssh_session", { sessionId: cur.id, data: cmd + "\n" });
  } else {
    navigator.clipboard.writeText(cmd);
  }
}

// 将备忘录命令填入当前 SSH 终端输入行，但不发送回车，因此不会执行。
function fillMemoCommandToTerminal(cmd: string) {
  const cur = sessions.value.find(s => s.id === activeSessionId.value);
  if (!cur || !cur.alive) return;
  // 只读模式禁止向远端写入任何内容，与粘贴/键盘输入保持一致。
  if (cur.read_only) return;
  // 只填充不执行：换行会被远端 shell 立即执行，
  // 所以多行命令只填充首行，并明确告知用户，避免"部分执行"。
  const normalized = cmd.replace(/\r\n/g, "\n").replace(/[\r\n]+$/, "");
  const lines = normalized.split("\n");
  const firstLine = (lines[0] || "").trim();
  if (lines.length > 1) {
    console.warn("[memo] 命令包含多行，已仅填充首行以避免意外执行");
    alert(`该命令包含 ${lines.length} 行，为安全起见已仅填充首行：\n${firstLine}`);
  }
  if (!firstLine) return;
  const writeCommand = isGenericProtocol(cur.host?.protocol) ? "write_generic_session" : "write_ssh_session";
  invoke(writeCommand, { sessionId: cur.id, data: firstLine }).catch((e: any) => {
    console.error("写入终端失败:", e);
    alert("写入终端失败: " + (e?.message || e));
  });
  cur.termInstance?.focus();
  closeContextMenu();
}

function openAddMemoModal(groupIdx?: number | null) {
  if (!configData.value.memos) configData.value.memos = [];

  let targetIndex = groupIdx;
  if (targetIndex === undefined || targetIndex === null || !configData.value.memos[targetIndex]) {
    targetIndex = configData.value.memos.findIndex(g => g.group === "未分组");
    if (targetIndex < 0) {
      configData.value.memos.push({ group: "未分组", items: [] });
      targetIndex = configData.value.memos.length - 1;
    }
  }

  memoEditModal.value = {
    show: true,
    isEdit: false,
    groupIndex: targetIndex,
    itemIndex: -1,
    form: { name: "", cmd: "" }
  };
}

function openEditMemoModal(groupIdx: number, itemIdx: number, item: MemoItem) {
  memoEditModal.value = {
    show: true,
    isEdit: true,
    groupIndex: groupIdx,
    itemIndex: itemIdx,
    form: { name: item.name, cmd: item.cmd }
  };
}

async function saveMemo() {
  const m = memoEditModal.value;
  if (!configData.value.memos) configData.value.memos = [];
  const group = configData.value.memos[m.groupIndex];
  if (!group) return;

  if (m.isEdit && m.itemIndex >= 0) {
    group.items[m.itemIndex] = { ...m.form };
  } else {
    group.items.push({ ...m.form });
  }

  await invoke("save_hosts_config", { config: configData.value });
  m.show = false;
}

async function deleteMemo(groupIdx: number, itemIdx: number) {
  if (!configData.value.memos) return;
  configData.value.memos[groupIdx].items.splice(itemIdx, 1);
  await invoke("save_hosts_config", { config: configData.value });
}

// --- 延迟测速 ---
async function testLatency(h: HostItem, mode: 'tcp' | 'icmp' = 'tcp') {
  h.latency_text = `${mode.toUpperCase()} 测速中...`;
  try {
    const portNum = Number(h.port) || 22;
    const res = await invoke<string>("probe_latency", {
      host: h.host.trim(),
      port: portNum,
      mode: mode,
    });
    h.latency_text = res;
    // 异步同步到配置文件中以便持久化上次测速状态
    invoke("save_hosts_config", { config: configData.value }).catch(() => {});
  } catch (e: any) {
    h.latency_text = `${mode.toUpperCase()} 失败`;
  }
}

// --- 终端剪贴板辅助 ---
async function readClipboardText(): Promise<string> {
  if (isTauri) {
    try {
      return await invoke<string>("read_clipboard_text");
    } catch (e) {
      console.warn("[clipboard] native read failed:", e);
    }
  }
  return navigator.clipboard.readText();
}

async function writeClipboardText(text: string): Promise<void> {
  if (!text) return;
  if (isTauri) {
    try {
      await invoke("write_clipboard_text", { text });
      return;
    } catch (e) {
      console.warn("[clipboard] native write failed:", e);
    }
  }
  await navigator.clipboard.writeText(text);
}

async function pasteIntoActiveTerminal() {
  const cur = sessions.value.find(s => s.id === activeSessionId.value);
  if (!cur || !cur.alive || cur.read_only) {
    closeContextMenu();
    // 早退也要把焦点还给终端，否则用户会以为"键盘失灵"。
    cur?.termInstance?.focus();
    return;
  }
  try {
    const text = await readClipboardText();
    if (text) {
      if (isGenericProtocol(cur.host?.protocol)) {
        await invoke("write_generic_session", { sessionId: cur.id, data: text });
      } else {
        await invoke("write_ssh_session", { sessionId: cur.id, data: text });
      }
    }
  } catch (e) {
    console.warn("[clipboard] paste failed:", e);
  } finally {
    closeContextMenu();
    cur.termInstance?.focus();
  }
}

function copySelection() {
  const cur = sessions.value.find(s => s.id === activeSessionId.value);
  const sel = cur?.termInstance?.getSelection() || "";
  closeContextMenu();
  void writeClipboardText(sel).finally(() => {
    cur?.termInstance?.focus();
  });
}

async function pasteClipboard() {
  await pasteIntoActiveTerminal();
}

function clearCurrentTerminal() {
  const cur = sessions.value.find(s => s.id === activeSessionId.value);
  if (cur?.termInstance) cur.termInstance.clear();
  closeContextMenu();
}

function selectAllTerminal() {
  const cur = sessions.value.find(s => s.id === activeSessionId.value);
  if (cur?.termInstance) {
    cur.termInstance.selectAll();
    cur.termInstance.focus();
  }
  closeContextMenu();
}

// --- 本地与 WebDAV 备份业务逻辑 ---
let webdavSaveTimer: number | null = null;

function applyWebdavSettingsToConfig() {
  if (!configData.value.settings) configData.value.settings = {};
  configData.value.settings.webdav = {
    url: backupModal.value.webdav_url,
    user: backupModal.value.webdav_user,
    pwd: backupModal.value.webdav_pwd,
    path: backupModal.value.webdav_path,
  };
}

function scheduleWebdavSettingsSave() {
  applyWebdavSettingsToConfig();
  if (webdavSaveTimer !== null) {
    clearTimeout(webdavSaveTimer);
  }
  webdavSaveTimer = window.setTimeout(() => {
    webdavSaveTimer = null;
    void invoke("save_hosts_config", { config: configData.value }).catch((error) => {
      console.warn("[webdav] 保存连接配置失败:", error);
    });
  }, 500);
}

async function persistWebdavSettings() {
  applyWebdavSettingsToConfig();
  if (webdavSaveTimer !== null) {
    clearTimeout(webdavSaveTimer);
    webdavSaveTimer = null;
  }
  await invoke("save_hosts_config", { config: configData.value });
}

async function handleExportLocal() {
  const savePath = await openLocalFileBrowser({
    action: "save",
    title: "导出完整配置备份包",
    defaultName: "QShell_Backup.zip",
  });
  if (!savePath) return;
  try {
    await persistWebdavSettings();
    const res = await invoke<{ success: boolean; message: string }>("export_local_backup", { targetPath: savePath });
    backupModal.value.status_msg = res.message;
  } catch (e: any) {
    backupModal.value.status_msg = `导出失败: ${e}`;
  }
}

async function handleImportLocal() {
  const openPath = await openLocalFileBrowser({
    action: "open",
    title: "选择导入的备份文件",
  });
  if (!openPath) return;
  const pathStr = openPath;
  try {
    await persistWebdavSettings();
    const res = await invoke<{ success: boolean; message: string }>("import_local_backup", { sourcePath: pathStr });
    backupModal.value.status_msg = res.message;
    await loadConfig();
  } catch (e: any) {
    backupModal.value.status_msg = `导入失败: ${e}`;
  }
}

async function handleWebdavTest() {
  backupModal.value.is_busy = true;
  backupModal.value.status_msg = "正在测试连接 WebDAV...";
  try {
    await persistWebdavSettings();
    const res = await invoke<{ success: boolean; message: string }>("test_webdav_connection", {
      cfg: {
        url: backupModal.value.webdav_url,
        user: backupModal.value.webdav_user,
        pwd: backupModal.value.webdav_pwd || null,
        path: backupModal.value.webdav_path,
      }
    });
    backupModal.value.status_msg = res.message;
  } catch (e: any) {
    backupModal.value.status_msg = `连接失败: ${e}`;
  } finally {
    backupModal.value.is_busy = false;
  }
}

async function handleWebdavUpload() {
  backupModal.value.is_busy = true;
  backupModal.value.status_msg = "正在上传备份至 WebDAV 云端...";
  try {
    await persistWebdavSettings();
    const res = await invoke<{ success: boolean; message: string }>("webdav_upload_backup", {
      cfg: {
        url: backupModal.value.webdav_url,
        user: backupModal.value.webdav_user,
        pwd: backupModal.value.webdav_pwd || null,
        path: backupModal.value.webdav_path,
      }
    });
    backupModal.value.status_msg = res.message;
  } catch (e: any) {
    backupModal.value.status_msg = `上传失败: ${e}`;
  } finally {
    backupModal.value.is_busy = false;
  }
}

async function handleWebdavDownload() {
  backupModal.value.is_busy = true;
  backupModal.value.status_msg = "正在从云端拉取备份恢复...";
  try {
    await persistWebdavSettings();
    const res = await invoke<{ success: boolean; message: string }>("webdav_download_backup", {
      cfg: {
        url: backupModal.value.webdav_url,
        user: backupModal.value.webdav_user,
        pwd: backupModal.value.webdav_pwd || null,
        path: backupModal.value.webdav_path,
      }
    });
    backupModal.value.status_msg = res.message;
    await loadConfig();
  } catch (e: any) {
    backupModal.value.status_msg = `恢复失败: ${e}`;
  } finally {
    backupModal.value.is_busy = false;
  }
}

// 批量从设备树导入到主机管理
function openImportSelectModal() {
  const allTreeHosts: HostItem[] = [];
  configData.value.devices.forEach(g => {
    g.hosts.forEach(h => {
      if (!configData.value.managed_hosts.some(m => m.host === h.host && m.port === h.port)) {
        allTreeHosts.push(h);
      }
    });
  });
  importSelectModal.value = {
    show: true,
    candidates: allTreeHosts,
    selectedIndexes: allTreeHosts.map((_, i) => i),
  };
}

async function confirmImportSelected() {
  const { candidates, selectedIndexes } = importSelectModal.value;
  selectedIndexes.forEach(idx => {
    configData.value.managed_hosts.push(candidates[idx]);
  });
  await invoke("save_hosts_config", { config: configData.value });
  importSelectModal.value.show = false;
}

// --- 生命周期 ---
let unlistenTransfer: UnlistenFn | null = null;
let unlistenCloseRequested: UnlistenFn | null = null;
let windowResizeEndTimer: number | null = null;
let resizeThrottleTimer: number | null = null;

// 具名 resize 处理器：onUnmounted 时才能正确解绑，避免监听泄漏
function handleWindowResize() {
  document.documentElement.classList.add("window-resizing");
  if (windowResizeEndTimer) window.clearTimeout(windowResizeEndTimer);
  windowResizeEndTimer = window.setTimeout(() => {
    document.documentElement.classList.remove("window-resizing");
    sessions.value.forEach(s => s.fitInstance?.fit());
  }, 120);

  // 智能自适应折叠：当窗口缩到过窄容纳不下三栏时，自动收起右栏避免挤出屏幕外被截断
  if (window.innerWidth < 820 && isRightPanelOpen.value) {
    isRightPanelOpen.value = false;
  }

  // 节流处理 fit() 计算，避免拖动窗口每一帧强刷所有活动/非活动终端实例
  if (!resizeThrottleTimer) {
    resizeThrottleTimer = window.setTimeout(() => {
      resizeThrottleTimer = null;
      // 仅适配当前前台活跃会话，非活跃会话切回时自适应，从而释放 90% CPU 占用
      const active = activeSession.value;
      if (active?.fitInstance) {
        try { active.fitInstance.fit(); } catch (_) {}
      }
    }, 50);
  }
}

// WebView2 的原生文件拖放与 HTML5 内部拖拽在 Windows 上互斥。
// 平时保留原生拖放用于 SFTP 拖文件上传；当按下带 draggable 的内部元素时
// 临时关闭原生拖放，让设备/备忘录排序的 HTML5 拖拽生效。
let externalDropSuppressed = false;

function setNativeExternalDrop(enabled: boolean) {
  if (!isTauri) return;
  invoke("set_external_drop_enabled", { enabled }).catch((error) => {
    console.warn("[drag] 切换原生文件拖放失败:", error);
  });
}

function suppressNativeDropForInternalDrag(event: PointerEvent) {
  const target = event.target as Element | null;
  if (!target?.closest('[draggable="true"]')) return;
  if (externalDropSuppressed) return;
  externalDropSuppressed = true;
  setNativeExternalDrop(false);
}

function restoreNativeDropAfterInternalDrag() {
  if (!externalDropSuppressed) return;
  externalDropSuppressed = false;
  setNativeExternalDrop(true);
}

onMounted(async () => {
  document.addEventListener("pointerdown", suppressNativeDropForInternalDrag, true);
  document.addEventListener("pointerup", restoreNativeDropAfterInternalDrag, true);
  document.addEventListener("dragend", restoreNativeDropAfterInternalDrag, true);
  document.addEventListener("drop", restoreNativeDropAfterInternalDrag, true);
  document.addEventListener("pointermove", onPointerDragMove);
  document.addEventListener("pointerup", onPointerDragEnd);
  document.addEventListener("pointercancel", onPointerDragCancel);
  document.addEventListener("click", suppressClickAfterPointerDrag, true);
  document.addEventListener("dblclick", suppressClickAfterPointerDrag, true);
  window.addEventListener("blur", restoreNativeDropAfterInternalDrag);
  // 拖拽中窗口失焦（Alt+Tab）会丢失 pointerup，必须主动清理拖拽状态，
  // 否则 pointerDrag 与 body 的 user-select:none 会一直残留。
  window.addEventListener("blur", onPointerDragCancel);

  await loadConfig();
  await refreshConfigEncryptionState();
  let launchedByAutostart = false;
  if (isTauri) {
    try {
      launchedByAutostart = await invoke<boolean>("is_autostart_launch");
      autostartEnabled.value = await invoke<boolean>("get_autostart_enabled");
    } catch (error) {
      console.warn("[autostart] 读取开机自启状态失败:", error);
    }
  }
  applyAccentColor(terminalSettings.value.accent_color);
  await nextTick();
  // 启动 8 秒后静默检查一次更新（开机自启时不打扰）
  if (isTauri && !launchedByAutostart) {
    window.setTimeout(() => {
      void checkForUpdates(true);
    }, 8000);
  }
  if (appWindow && !launchedByAutostart) {
    try {
      await appWindow.show();
      await appWindow.setFocus();
      await invoke("apply_window_rounding").catch(() => {});
    } catch (e) {
      console.warn("[window] 显示主窗口失败:", e);
    }
  }
  startTrafficAuditScheduler();
  window.addEventListener("focus", handleTrafficSchedulerWake);
  document.addEventListener("visibilitychange", handleTrafficSchedulerWake);
  window.addEventListener("click", handleBaudOutsideClick);
  window.addEventListener("qshell-close-selects", handleCloseSelects);
  window.addEventListener("click", handleGlobalClick);
  window.addEventListener("resize", handleWindowResize);

  // 监听 SFTP 文件传输进度广播 (仅在 Tauri 运行环境下监听)
  if (isTauri) {
    unlistenTransfer = await listen<TransferTask>("sftp-transfer-progress", (event) => {
      const payload = event.payload;
      const existing = activeTransfers.value.find(t => t.task_id === payload.task_id);
      if (existing) {
        Object.assign(existing, payload);
      } else {
        activeTransfers.value.push(payload);
      }
    });
  }

  // 监听 Tauri 原生拖拽文件到窗口释放事件
  if (appWindow) {
    await appWindow.onDragDropEvent((event) => {
      if (event.payload.type === "over") {
        if (rightPanelTab.value === "sftp" && canUseSftp.value) {
          isSftpDragOver.value = true;
        }
      } else if (event.payload.type === "drop") {
        isSftpDragOver.value = false;
        if (rightPanelTab.value === "sftp" && canUseSftp.value && event.payload.paths) {
          event.payload.paths.forEach((p: string) => {
            uploadLocalPath(p);
          });
        }
      } else {
        isSftpDragOver.value = false;
      }
    });

    // 关闭按钮行为：开启“隐藏到托盘”时阻止真正退出，只隐藏窗口
    // 透明窗口缩放时先铺主题底色，避免 WebView2 默认白底从圆角处漏出
    await appWindow.onResized(() => {
      document.documentElement.classList.add("window-resizing");
      if (windowResizeEndTimer) window.clearTimeout(windowResizeEndTimer);
      windowResizeEndTimer = window.setTimeout(() => {
        document.documentElement.classList.remove("window-resizing");
      }, 180);
    });

    unlistenCloseRequested = await appWindow.onCloseRequested(async (event) => {
      event.preventDefault();
      if (terminalSettings.value.minimize_to_tray) {
        try {
          await invoke("notify_tray", {
            body: "QShell 已隐藏到系统托盘，点击托盘图标可重新打开。",
          });
        } catch (notifyError) {
          console.warn("[tray] 发送系统通知失败:", notifyError);
        }
        await appWindow?.hide();
      } else {
        // 让 WebView2 正常销毁窗口，Rust 侧在窗口销毁后再退出进程
        await appWindow?.destroy();
      }
    });
  }

  // 读取上次窗口状态
  try {
    const savedState = localStorage.getItem("qshell_window_state");
    if (savedState) {
      const parsed = JSON.parse(savedState);
      if (parsed.isRightPanelOpen !== undefined) isRightPanelOpen.value = parsed.isRightPanelOpen;
      if (parsed.currentView) currentView.value = parsed.currentView;
    }
  } catch (e) {}
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", suppressNativeDropForInternalDrag, true);
  document.removeEventListener("pointerup", restoreNativeDropAfterInternalDrag, true);
  document.removeEventListener("dragend", restoreNativeDropAfterInternalDrag, true);
  document.removeEventListener("drop", restoreNativeDropAfterInternalDrag, true);
  document.removeEventListener("pointermove", onPointerDragMove);
  document.removeEventListener("pointerup", onPointerDragEnd);
  document.removeEventListener("pointercancel", onPointerDragCancel);
  document.removeEventListener("click", suppressClickAfterPointerDrag, true);
  document.removeEventListener("dblclick", suppressClickAfterPointerDrag, true);
  window.removeEventListener("blur", restoreNativeDropAfterInternalDrag);
  window.removeEventListener("blur", onPointerDragCancel);
  window.removeEventListener('click', handleBaudOutsideClick);
  window.removeEventListener("qshell-close-selects", handleCloseSelects);
  window.removeEventListener("click", handleGlobalClick);
  window.removeEventListener("resize", handleWindowResize);
  window.removeEventListener("focus", handleTrafficSchedulerWake);
  document.removeEventListener("visibilitychange", handleTrafficSchedulerWake);
  if (unlistenCloseRequested) unlistenCloseRequested();
  stopTrafficAuditScheduler();
  if (unlistenTransfer) unlistenTransfer();
  sessions.value.forEach(s => s.termInstance?.dispose());
  try {
    localStorage.setItem("qshell_window_state", JSON.stringify({
      isRightPanelOpen: isRightPanelOpen.value,
      currentView: currentView.value,
    }));
  } catch (e) {}
});
</script>

<template>
  <div class="app-root" @contextmenu.prevent>
    <!-- 顶部标题栏 (1:1 像素级复刻原版设计) -->
    <header class="title-bar" @mousedown="startDragging" @dblclick="onTitleBarDoubleClick">
      <div class="brand">
        <span class="logo-title">QShell</span>
        <div class="tag-version-wrapper">
          <div class="version-badge">
            <span class="badge-dot">●</span>
            <span class="badge-text">v{{ appVersion }}</span>
          </div>
        </div>
      </div>

      <!-- 右侧控制区：主机管理药丸按钮 + 平铺矢量图标操作栏 + 窗口控制 -->
      <div class="top-right-group">
        <button 
          :class="['btn-mgmt-pill', currentView === 'mgmt' ? 'active' : '']"
          @click="currentView = currentView === 'workspace' ? 'mgmt' : 'workspace'"
        >
          {{ currentView === 'mgmt' ? '返回主页' : '主机管理' }}
        </button>

        <button class="top-icon-btn" :title="themeMode === 'dark' ? '切换为浅色模式' : '切换为深色模式'" @click="toggleTheme($event)">
          <Icons :name="themeMode === 'dark' ? 'moon' : 'sun'" size="24" />
        </button>
        
        <button 
          :class="['top-icon-btn', 'btn-toggle-alert', hostAlertList.length > 0 ? 'has-alert' : '', showAlertPopover ? 'active' : '']"
          @click="toggleAlertPopover($event)"
          title="通知 / 到期与流量预警"
        >
          <Icons name="bell" size="24" />
          <span v-if="hostAlertList.length > 0" class="alert-dot"></span>
        </button>

        <button 
          :class="['top-icon-btn', 'btn-toggle-transfer', showTransferMonitor ? 'active' : '']"
          @click="toggleTransferMonitor($event)"
          title="SFTP 文件传输任务"
        >
          <Icons name="arrows" size="24" />
        </button>

        <button 
          :class="['top-icon-btn', showTerminalSettingsModal ? 'active' : '']"
          @click="showTerminalSettingsModal = true"
          title="首选项与终端设置"
        >
          <Icons name="settings" size="24" />
        </button>

        <button 
          :class="['top-icon-btn', isRightPanelOpen ? 'active' : '']"
          @click="isRightPanelOpen = !isRightPanelOpen"
          :title="isRightPanelOpen ? '折叠右侧栏' : '展开右侧栏'"
        >
          <Icons name="panel" size="24" />
        </button>

        <!-- 窗口原生控制按钮 (34px × 30px，1.6px 饱满线宽) -->
        <div class="window-controls">
          <button class="win-btn" title="最小化" @click.stop="minimizeWindow">
            <Icons name="win-min" size="14" />
          </button>
          <button class="win-btn" id="snap-btn" title="最大化" @click.stop="toggleMaximizeWindow">
            <Icons name="win-max" size="14" />
          </button>
          <button class="win-btn close" title="关闭" @click.stop="closeWindow">
            <Icons name="win-close" size="14" />
          </button>
        </div>
      </div>
    </header>

    <!-- 主工作区：经典三栏架构 (左系统监控与设备树 / 中终端多标签 / 右SFTP与备忘录) -->
    <div class="main-body">
      <!-- 视图 1：工作区 (使用不可见定位层隐藏，彻底避免 display:none 导致 xterm 几何尺寸归零与字符吞并) -->
      <div :class="['workspace-layout', currentView !== 'workspace' ? 'view-hidden' : '']">
        
        <!-- 【左栏】：系统监控卡片 + 设备列表卡片 (独立圆角卡片，支持拖拽调节宽度) -->
        <aside class="left-panel" :style="{ width: leftPanelWidth + 'px', minWidth: leftPanelWidth + 'px' }">
          <!-- 上卡片：系统监控卡片 (自适应自然内容高度，严丝合缝无死板空隙) -->
          <div class="card-container monitor-card" :style="{ minHeight: leftMonitorCardHeight + 'px' }">
            <div class="card-header-title">系统监控</div>
            
            <div class="gauge-group">
              <div class="gauge-item">
                <div class="gauge-label">
                  <span class="g-name">CPU 使用率</span>
                  <span class="g-val">{{ monitorStats.cpu }}%</span>
                </div>
                <div class="gauge-track">
                  <div class="gauge-fill cpu" :style="{ width: monitorStats.cpu + '%', minWidth: monitorStats.cpu > 0 ? '6px' : '0' }"></div>
                </div>
                <div class="metric-detail">
                  <span class="metric-model" :title="monitorStats.cpu_model || '未知型号'">{{ monitorStats.cpu_model || '未知型号' }}</span>
                  <span v-if="monitorStats.cpu_cores" class="metric-chip">{{ monitorStats.cpu_cores }}C</span>
                </div>
              </div>

              <div class="gauge-item">
                <div class="gauge-label">
                  <span class="g-name">内存占用</span>
                  <span class="g-val">{{ monitorStats.mem }}%</span>
                </div>
                <div class="gauge-track">
                  <div class="gauge-fill mem" :style="{ width: monitorStats.mem + '%', minWidth: monitorStats.mem > 0 ? '6px' : '0' }"></div>
                </div>
                <div class="metric-detail">
                  <span>{{ formatMetricBytes(monitorStats.mem_used_bytes) }} / {{ formatMetricBytes(monitorStats.mem_total_bytes) }}</span>
                </div>
              </div>

              <div class="gauge-item">
                <div class="gauge-label">
                  <span class="g-name">硬盘总占用</span>
                  <span class="g-val">{{ monitorStats.disk }}%</span>
                </div>
                <div class="gauge-track">
                  <div class="gauge-fill disk" :style="{ width: monitorStats.disk + '%', minWidth: monitorStats.disk > 0 ? '6px' : '0' }"></div>
                </div>
                <div class="metric-detail">
                  <span>{{ formatMetricBytes(monitorStats.disk_used_bytes) }} / {{ formatMetricBytes(monitorStats.disk_total_bytes) }}</span>
                </div>
              </div>
            </div>

            <!-- 网络吞吐区：网卡状态放在标题右侧，不再单独占一行 -->
            <div class="net-section-head" @click="showNetIfPicker = false">
              <span class="net-section-title">网络吞吐</span>
              <button
                type="button"
                class="net-view-toggle"
                @click.stop="networkView = networkView === 'live' ? 'stats' : 'live'"
                :title="networkView === 'live' ? '切换到流量统计' : '切换到实时视图'"
              >{{ networkView === 'live' ? '统计' : '实时' }}</button>
              <div class="net-if-wrap" @click.stop>
                <button
                  type="button"
                  :class="['net-if-badge', monitorStats.net_if ? 'connected' : 'idle']"
                  @click="showNetIfPicker = !showNetIfPicker"
                  title="切换网卡"
                >
                  <span class="if-dot">●</span>
                  <span class="if-text">{{ monitorStats.net_if ? `已连接: ${monitorStats.net_if}` : (sessions.length > 0 ? '检测网卡中...' : '网络未连接') }}</span>
                  <span class="if-chevron">⌄</span>
                </button>
                <div v-if="showNetIfPicker" class="net-if-menu">
                  <button
                    v-for="name in availableNetInterfaces"
                    :key="name"
                    type="button"
                    :class="['net-if-option', monitorStats.net_if === name ? 'active' : '']"
                    @click="selectNetInterface(name)"
                  >
                    <span class="if-option-dot"></span>
                    <span>{{ name }}</span>
                    <span v-if="monitorStats.net_if === name" class="if-option-check">✓</span>
                  </button>
                  <div v-if="availableNetInterfaces.length === 0" class="net-if-empty">暂无可用网卡</div>
                </div>
              </div>
            </div>

            <!-- 网络面板：正方形，实时/统计切换 -->
            <div class="net-panel">
            <div v-if="networkView === 'live'" class="net-live-view">
              <div class="net-sub-card">
                <div class="net-header">
                  <span class="net-direction up">↑</span>
                  <span class="net-label">发送</span>
                  <span class="net-speed">{{ monitorStats.net_up }}</span>
                </div>
                <div class="led-svg-wrap">
                  <svg viewBox="0 0 108 60" class="led-matrix-svg" preserveAspectRatio="xMidYMid meet">
                    <template v-for="row in 10" :key="'up-row-' + row">
                      <rect
                        v-for="col in 18"
                        :key="'up-rect-' + ((row - 1) * 18 + col)"
                        :x="(col - 1) * 6"
                        :y="(row - 1) * 6"
                        width="6"
                        height="6"
                        rx="1.3"
                        :opacity="ledFadeOpacity((10 - row) * 18 + col, activeUpLedCount)"
                        :class="[
                          'svg-led',
                          ((10 - row) * 18 + col) <= activeUpLedCount ? 'active-up' : '',
                          getRunnerLevel((10 - row) * 18 + col, upRunnerIdx, activeUpLedCount) === 0 ? 'runner-head-up' : '',
                          getRunnerLevel((10 - row) * 18 + col, upRunnerIdx, activeUpLedCount) === 1 ? 'runner-tail-1-up' : '',
                          getRunnerLevel((10 - row) * 18 + col, upRunnerIdx, activeUpLedCount) === 2 ? 'runner-tail-2-up' : '',
                          getRunnerLevel((10 - row) * 18 + col, upRunnerIdx, activeUpLedCount) === 3 ? 'runner-tail-3-up' : ''
                        ]"
                      />
                    </template>
                  </svg>
                </div>
              </div>

              <div class="net-sub-card">
                <div class="net-header">
                  <span class="net-direction down">↓</span>
                  <span class="net-label">接收</span>
                  <span class="net-speed">{{ monitorStats.net_down }}</span>
                </div>
                <div class="led-svg-wrap">
                  <svg viewBox="0 0 108 60" class="led-matrix-svg" preserveAspectRatio="xMidYMid meet">
                    <template v-for="row in 10" :key="'down-row-' + row">
                      <rect
                        v-for="col in 18"
                        :key="'down-rect-' + ((row - 1) * 18 + col)"
                        :x="(col - 1) * 6"
                        :y="(row - 1) * 6"
                        width="6"
                        height="6"
                        rx="1.3"
                        :opacity="ledFadeOpacity((10 - row) * 18 + col, activeDownLedCount)"
                        :class="[
                          'svg-led',
                          ((10 - row) * 18 + col) <= activeDownLedCount ? 'active-down' : '',
                          getRunnerLevel((10 - row) * 18 + col, downRunnerIdx, activeDownLedCount) === 0 ? 'runner-head-down' : '',
                          getRunnerLevel((10 - row) * 18 + col, downRunnerIdx, activeDownLedCount) === 1 ? 'runner-tail-1-down' : '',
                          getRunnerLevel((10 - row) * 18 + col, downRunnerIdx, activeDownLedCount) === 2 ? 'runner-tail-2-down' : '',
                          getRunnerLevel((10 - row) * 18 + col, downRunnerIdx, activeDownLedCount) === 3 ? 'runner-tail-3-down' : ''
                        ]"
                      />
                    </template>
                  </svg>
                </div>
              </div>
            </div>
            <div v-else class="net-stats-view">
              <div class="net-stats-legend">
                <span class="net-stats-up">↑ {{ monitorStats.net_up }}</span>
                <span class="net-stats-down">↓ {{ monitorStats.net_down }}</span>
              </div>
              <div class="net-stats-chart">
                <svg viewBox="0 0 230 80" preserveAspectRatio="none" class="net-chart-svg">
                  <line x1="28" y1="6" x2="224" y2="6" class="chart-grid" />
                  <line x1="28" y1="33" x2="224" y2="33" class="chart-grid" />
                  <line x1="28" y1="60" x2="224" y2="60" class="chart-grid chart-axis" />
                  <text x="24" y="9" text-anchor="end" class="chart-tick">{{ formatAxisSpeed(networkRateMax) }}</text>
                  <text x="24" y="36" text-anchor="end" class="chart-tick">{{ formatAxisSpeed(networkRateMax / 2) }}</text>
                  <text x="24" y="63" text-anchor="end" class="chart-tick">0</text>
                  <polyline :points="buildNetworkPolyline('up')" class="chart-line-up" />
                  <polyline :points="buildNetworkPolyline('down')" class="chart-line-down" />
                  <text x="28" y="74" class="chart-tick">-3m</text>
                  <text x="224" y="74" text-anchor="end" class="chart-tick">现在</text>
                </svg>
              </div>
            </div>
          </div>
          </div>

          <!-- 左栏上下垂直调节手柄 (1:1 对标 ssh8.py self.left_splitter) -->
          <div class="splitter-handle-v" @mousedown="onLeftVerticalSplitterDown"></div>

          <!-- 下卡片：设备列表独立卡片 -->
          <div class="card-container device-list-card" @contextmenu.prevent="openContextMenu($event, 'group')">
            <div class="card-header-title">设备列表</div>

            <!-- 实时过滤搜索条 (高清晰矢量镜 + 居中防裁切) -->
            <div class="search-box-row">
              <span class="search-icon-slot">
                <Icons name="search" size="14" />
              </span>
              <input 
                v-model="searchKeyword" 
                class="device-search-input" 
                placeholder="搜索主机名/IP..." 
              />
              <span v-if="searchKeyword" class="search-clear" @click="searchKeyword = ''">✕</span>
            </div>
            
            <div class="tree-scroll">
              <div 
                v-for="(g, gIdx) in configData.devices" 
                :key="g.group" 
                class="group-wrapper"
                :data-device-group="gIdx"
              >
                <!-- 分组标题行：可拖拽调整分组顺序，也可以接收从其他分组拖过来的主机 -->
                <div 
                  class="group-header" 
                  :draggable="false"
                  @pointerdown="beginPointerDrag('device_group', gIdx, null, $event)"
                  :class="{
                    'drag-target-before': dragState.type === 'device_group' && dragState.targetGroupIdx === gIdx && dragState.targetPosition === 'before',
                    'drag-target-after': dragState.type === 'device_group' && dragState.targetGroupIdx === gIdx && dragState.targetPosition === 'after',
                    'drag-target-inside': dragState.type === 'host' && dragState.targetGroupIdx === gIdx && dragState.targetPosition === 'inside',
                    'is-dragging': dragState.type === 'device_group' && dragState.sourceGroupIdx === gIdx
                  }"
                  @dragstart="onDeviceGroupDragStart(gIdx, $event)"
                  @dragover="onDeviceGroupDragOver(gIdx, $event)"
                  @drop="onDeviceGroupDrop(gIdx, $event)"
                  @click="g.collapsed = !g.collapsed" 
                  @contextmenu.prevent="openContextMenu($event, 'group', undefined, g)"
                >
                  <Icons name="chevron" :expanded="!g.collapsed" size="12" class="group-chevron" />
                  <Icons name="folder" size="20" class="group-folder-icon" />
                  <span class="group-name">{{ g.group }}</span>
                  <span class="group-badge">({{ g.hosts?.length || 0 }})</span>
                </div>

                <!-- 分组内容列表：支持主机条目排序与跨分组插入 -->
                <Transition name="accordion">
                  <div 
                    v-show="!g.collapsed" 
                    class="group-body"
                    @dragover.prevent="dragState.type === 'host' && (dragState.targetGroupIdx = gIdx, dragState.targetItemIdx = null, dragState.targetPosition = 'inside')"
                    @drop="onHostDrop(gIdx, null, $event)"
                  >
                    <div 
                      v-for="(h, hIdx) in g.hosts" 
                      v-show="!searchKeyword || h.name.toLowerCase().includes(searchKeyword.toLowerCase()) || h.host.includes(searchKeyword)"
                      :key="h.host + h.port"
                      class="host-capsule-item"
                      :class="{
                        'drag-target-before': dragState.type === 'host' && dragState.targetGroupIdx === gIdx && dragState.targetItemIdx === hIdx && dragState.targetPosition === 'before',
                        'drag-target-after': dragState.type === 'host' && dragState.targetGroupIdx === gIdx && dragState.targetItemIdx === hIdx && dragState.targetPosition === 'after',
                        'is-dragging': dragState.type === 'host' && dragState.sourceGroupIdx === gIdx && dragState.sourceItemIdx === hIdx
                      }"
                      :data-host-group="gIdx"
                      :data-host-index="hIdx"
                      :draggable="false"
                      @pointerdown="beginPointerDrag('host', gIdx, hIdx, $event)"
                      @dragstart="onHostDragStart(gIdx, hIdx, $event)"
                      @dragover.stop="onHostDragOver(gIdx, hIdx, $event)"
                      @drop.stop="onHostDrop(gIdx, hIdx, $event)"
                      @dblclick="openSession(h)"
                      @contextmenu.prevent="openContextMenu($event, 'host', h, g)"
                    >
                      <!-- 左侧彩色胶囊指示条 -->
                      <span class="capsule-pillar"></span>
                      <Icons name="host" size="16" class="capsule-icon" />
                      <span class="capsule-title">{{ h.name }}</span>
                    </div>
                  </div>
                </Transition>
              </div>
            </div>
          </div>
        </aside>

        <!-- 左侧拖拽分割调节手柄 -->
        <div class="splitter-handle" @mousedown="onLeftSplitterDown"></div>

        <!-- 【中栏】：独立圆角大卡片 (包含多标签与终端渲染区) -->
        <main class="center-terminal-card">
          <div class="terminal-tab-bar" v-if="sessions.length > 0">
            <div class="tab-list">
              <div 
                v-for="(tab, tIdx) in sessions" 
                :key="tab.id"
                :class="[
                  'term-tab', 
                  activeSessionId === tab.id ? 'active' : '', 
                  tab.is_locked ? 'locked' : '',
                  draggingTabIndex === tIdx ? 'is-tab-dragging' : '',
                  dragOverTabIndex === tIdx && draggingTabIndex !== tIdx ? 'is-tab-drag-over' : ''
                ]"
                :data-tab-index="tIdx"
                :draggable="false"
                @pointerdown="beginPointerDrag('tab', tIdx, null, $event)"
                @click="switchSession(tab.id)"
                @contextmenu.prevent="openContextMenu($event, 'tab', undefined, undefined, undefined, undefined, undefined, undefined, tab)"
              >
                <span :class="['tab-dot', tab.alive ? 'online' : 'offline']"></span>
                <span class="tab-title">{{ tab.name }}</span>
                <!-- 1:1 对标 ssh8.py CustomTabCloseButton: 锁定显示锁，未锁定显示关闭叉号 (右键可切换锁定) -->
                <span 
                  :class="['custom-tab-btn', tab.is_locked ? 'btn-lock' : 'btn-close']"
                  :title="tab.is_locked ? '此标签已锁定保护 (双击或右键解锁)' : '关闭标签页'"
                  @pointerdown.stop
                  @click.stop="tab.is_locked ? toggleTabLock(tab) : closeSession(tab)"
                >
                  <Icons v-if="tab.is_locked" name="lock" size="15" />
                  <span v-else class="btn-close-x">✕</span>
                </span>
              </div>
            </div>

            <div class="tab-actions">
              <div 
                v-if="activeSession"
                :class="['terminal-latency-badge', latencyStatusClass]"
                :title="latencyDisplayText"
              >
                <span class="badge-dot"></span>
                <span class="badge-text">{{ latencyDisplayText }}</span>
              </div>
              <button 
                v-if="activeSession"
                class="btn-danger-disconnect" 
                @click="closeSession(activeSession)"
              >
                断开连接
              </button>
            </div>
          </div>

          <div 
            class="terminal-viewport"
            @contextmenu.prevent="openContextMenu($event, 'terminal')"
          >
            <div v-if="activeSession?.read_only" class="terminal-readonly-badge">只读模式</div>
            <div v-if="sessions.length === 0" class="empty-state">
              <div class="empty-icon-wrap">
                <svg class="empty-desktop-icon" viewBox="0 0 64 64" fill="none" xmlns="http://www.w3.org/2000/svg">
                  <!-- 显示器外框 -->
                  <rect x="6" y="8" width="52" height="36" rx="5" class="svg-stroke" stroke-width="2.5" />
                  <!-- 屏幕内显示区 -->
                  <rect x="10" y="12" width="44" height="28" rx="2.5" class="svg-screen" />
                  <!-- 终端指示符 >_ -->
                  <path d="M16 21L21 26L16 31" class="svg-symbol" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
                  <line x1="25" y1="31" x2="33" y2="31" class="svg-symbol" stroke-width="2" stroke-linecap="round" />
                  <!-- 支架立柱 -->
                  <path d="M27 44V51C27 51.5 27.5 52 28 52H36C36.5 52 37 51.5 37 51V44" class="svg-stroke" stroke-width="2.5" stroke-linejoin="round" />
                  <!-- 底座 -->
                  <path d="M19 56H45" class="svg-stroke" stroke-width="2.5" stroke-linecap="round" />
                </svg>
              </div>
              <div class="empty-title">终端空闲</div>
              <div class="empty-hint">双击左侧设备列表以连接</div>
            </div>

            <div 
              v-for="tab in sessions" 
              :key="tab.id"
              :id="'term-' + tab.id"
              :class="['terminal-mount-box', { 'is-active': activeSessionId === tab.id }]"
            ></div>
          </div>
        </main>

        <!-- 右侧拖拽分割调节手柄 -->
        <Transition name="drawer-fade">
          <div v-show="isRightPanelOpen" class="splitter-handle" @mousedown="onRightSplitterDown"></div>
        </Transition>

        <!-- 【右栏】：独立圆角大卡片 (SFTP与备忘录，支持拖拽调节宽度) -->
        <aside 
          class="card-container right-panel-card" 
          :class="{ 'is-collapsed': !isRightPanelOpen }"
          :style="{ 
            width: isRightPanelOpen ? (rightPanelWidth + 'px') : '0px', 
            flexBasis: isRightPanelOpen ? (rightPanelWidth + 'px') : '0px' 
          }"
          @transitionend="onDrawerTransitionEnd"
        >
            <div class="right-card-header">
              <span class="right-card-title">{{ rightPanelTab === 'memo' ? '命令备忘录' : 'SFTP 远程文件' }}</span>
              <button 
                :class="['btn-switch-white-pill', rightPanelTab === 'sftp' ? 'btn-danger' : '']"
                @click="switchRightPanelTab(rightPanelTab === 'memo' ? 'sftp' : 'memo')"
              >
                {{ rightPanelTab === 'memo' ? 'SFTP 传输' : '关闭' }}
              </button>
            </div>

          <!-- SFTP 视图 (1:1 复刻原版 ssh8.py 设计) -->
          <div v-if="rightPanelTab === 'sftp'" class="sftp-container" @contextmenu.prevent="openContextMenu($event, 'sftp_bg')">
            <!-- 顶部快捷路径导航栏 -->
            <div class="sftp-nav-bar">
              <button class="btn-sftp-action" :disabled="!canUseSftp" @click="sftpNavigateParent" title="返回上一级">上一级</button>
              <input class="sftp-path-box" :disabled="!canUseSftp" v-model="currentSftpPath" @keyup.enter="canUseSftp && fetchSftpFiles(currentSftpPath)" />
              <button class="btn-sftp-action" :disabled="!canUseSftp" @click="fetchSftpFiles(currentSftpPath)" title="跳转/刷新">跳转</button>
            </div>

            <!-- 表头：名称 | 大小 | 权限 (支持拖拽调节宽度) -->
            <div class="sftp-table-header">
              <div class="col-name" :style="{ width: sftpColWidths.name + 'px' }">
                <span class="col-sort" @click.stop="toggleSftpSort('name')" title="按名称排序">
                  名称
                  <span v-if="sftpSortKey === 'name'" class="sort-arrow">{{ sftpSortAsc ? '↑' : '↓' }}</span>
                </span>
                <span class="col-resizer" @mousedown="onSftpColResizeDown('name', $event)" title="按住拖拽调节列宽">
                  <span class="resizer-dots"></span>
                </span>
              </div>
              <div class="col-size" :style="{ width: sftpColWidths.size + 'px' }">
                <span class="col-sort" @click.stop="toggleSftpSort('size')" title="按大小排序">
                  大小
                  <span v-if="sftpSortKey === 'size'" class="sort-arrow">{{ sftpSortAsc ? '↑' : '↓' }}</span>
                </span>
                <span class="col-resizer" @mousedown="onSftpColResizeDown('size', $event)" title="按住拖拽调节列宽">
                  <span class="resizer-dots"></span>
                </span>
              </div>
              <div class="col-perm">
                <span>权限</span>
              </div>
            </div>
            
            <div :class="['sftp-file-list', isSftpDragOver ? 'drag-over' : '']" :style="{ opacity: isSftpLoading ? 0.6 : 1, transition: 'opacity 0.2s' }" role="listbox" aria-multiselectable="true">
              <div v-if="isSftpDragOver" class="sftp-drop-overlay">
                释放文件立即上传到当前目录
              </div>
              <div v-if="!activeSession" class="sftp-empty">
                <span class="sftp-empty-title">请先连接 SSH 主机</span>
                <span class="sftp-empty-desc">连接后可浏览远程目录、上传和下载文件</span>
              </div>
              <div v-else-if="!activeSessionIsSsh" class="sftp-empty">
                <span class="sftp-empty-title">当前协议不支持 SFTP 文件传输</span>
                <span class="sftp-empty-desc">仅 SSH 协议网络终端支持 SFTP 远程文件浏览与拖拽上传</span>
              </div>
              <div v-else-if="!activeSession.alive" class="sftp-empty">
                <span class="sftp-empty-title">
                  {{ activeSession.is_connecting ? 'SSH 会话连接中...' : 'SSH 会话已断开' }}
                </span>
                <span class="sftp-empty-desc">
                  {{ activeSession.is_connecting ? '连接成功后将自动读取远程目录' : '请切换到其他标签页，或重新连接该主机' }}
                </span>
              </div>
              <div v-else-if="isSftpLoading && sftpFiles.length === 0" class="sftp-loading">读取目录中...</div>
              <div v-else-if="sftpErrorMsg" class="sftp-empty" style="color: #EF4444;">
                {{ sftpErrorMsg }}
              </div>
              <div v-else-if="sftpFiles.length === 0" class="sftp-empty">
                {{ activeSessionId && activeSession?.alive ? '目录为空或无权限' : '请先在左侧双击连接主机' }}
              </div>
              <div 
                v-for="(f, idx) in sftpFiles" 
                :key="f.name"
                :class="['file-item', f.is_dir ? 'dir' : 'file', isSftpSelected(f) ? 'selected' : '']"
                tabindex="0"
                role="option"
                :aria-selected="isSftpSelected(f)"
                @click="handleSftpItemClick(f, idx, $event)"
                @dblclick="sftpCd(f)"
                @keydown="handleSftpItemKeydown($event, f, idx)"
                @contextmenu.prevent="handleSftpContextMenu($event, f, idx)"
              >
                <div class="f-info" :style="{ width: sftpColWidths.name + 'px' }">
                  <Icons :name="f.is_dir ? 'folder' : 'file'" size="20" class="flex-shrink-0" />
                  <span class="f-name" :title="f.name">{{ f.name }}</span>
                </div>
                <div class="f-meta">
                  <span class="f-size" :style="{ width: sftpColWidths.size + 'px' }">{{ f.size }}</span>
                  <span class="f-perm">{{ f.mode_str }}</span>
                </div>
              </div>
            </div>

            <!-- 底部常驻双按钮: 上传文件 与 下载选中 (矢量图标替换) -->
            <div class="sftp-bottom-bar">
              <button class="btn-sftp-footer" :disabled="!canUseSftp" @click="sftpTriggerUpload">
                <Icons name="arrow-up" size="14" />
                <span>上传文件</span>
              </button>
              <button class="btn-sftp-footer" :disabled="!canUseSftp || selectedSftpFiles.filter(f => !f.is_dir).length === 0" @click="sftpTriggerDownloadSelected">
                <Icons name="arrow-down" size="14" />
                <span>下载选中{{ selectedSftpFiles.filter(f => !f.is_dir).length > 1 ? ` (${selectedSftpFiles.filter(f => !f.is_dir).length})` : '' }}</span>
              </button>
            </div>
          </div>

          <!-- 备忘录视图 (复刻原版圆角胶囊条与底部浅蓝添加按钮) -->
          <div v-else class="memo-container" @contextmenu.prevent="openContextMenu($event, 'memo_bg')">
            <div 
              v-for="(g, gIdx) in configData.memos" 
              :key="g.group" 
              class="memo-group-section"
              :data-memo-group="gIdx"
            >
              <!-- 备忘录分组头：可拖拽调整分组顺序，也可以接收从其他分组拖过来的备忘录条目 -->
              <div 
                class="memo-group-header" 
                :draggable="false"
                @pointerdown="beginPointerDrag('memo_group', gIdx, null, $event)"
                :class="{
                  'drag-target-before': dragState.type === 'memo_group' && dragState.targetGroupIdx === gIdx && dragState.targetPosition === 'before',
                  'drag-target-after': dragState.type === 'memo_group' && dragState.targetGroupIdx === gIdx && dragState.targetPosition === 'after',
                  'drag-target-inside': dragState.type === 'memo_item' && dragState.targetGroupIdx === gIdx && dragState.targetPosition === 'inside',
                  'is-dragging': dragState.type === 'memo_group' && dragState.sourceGroupIdx === gIdx
                }"
                @dragstart="onMemoGroupDragStart(gIdx, $event)"
                @dragover="onMemoGroupDragOver(gIdx, $event)"
                @drop="onMemoGroupDrop(gIdx, $event)"
                @click="g.collapsed = !g.collapsed"
                @contextmenu.prevent="openContextMenu($event, 'memo_group', undefined, undefined, undefined, gIdx)"
              >
                <Icons name="chevron" :expanded="!g.collapsed" size="12" class="group-chevron" />
                <Icons name="folder" size="20" class="memo-folder-icon" />
                <span class="memo-group-title">{{ g.group }}</span>
              </div>

              <!-- 备忘录列表：支持条目拖拽排序与跨组插入 -->
              <Transition name="accordion">
                <div 
                  v-show="!g.collapsed"
                  class="memo-list"
                  @dragover.prevent="dragState.type === 'memo_item' && (dragState.targetGroupIdx = gIdx, dragState.targetItemIdx = null, dragState.targetPosition = 'inside')"
                  @drop="onMemoDrop(gIdx, null, $event)"
                >
                  <div 
                    v-for="(item, iIdx) in g.items" 
                    :key="item.name + item.cmd" 
                    class="memo-capsule-item"
                    :class="{
                      'drag-target-before': dragState.type === 'memo_item' && dragState.targetGroupIdx === gIdx && dragState.targetItemIdx === iIdx && dragState.targetPosition === 'before',
                      'drag-target-after': dragState.type === 'memo_item' && dragState.targetGroupIdx === gIdx && dragState.targetItemIdx === iIdx && dragState.targetPosition === 'after',
                      'is-dragging': dragState.type === 'memo_item' && dragState.sourceGroupIdx === gIdx && dragState.sourceItemIdx === iIdx
                    }"
                    :data-memo-group="gIdx"
                    :data-memo-index="iIdx"
                    :draggable="false"
                    @pointerdown="beginPointerDrag('memo_item', gIdx, iIdx, $event)"
                    @dragstart="onMemoDragStart(gIdx, iIdx, $event)"
                    @dragover.stop="onMemoDragOver(gIdx, iIdx, $event)"
                    @drop.stop="onMemoDrop(gIdx, iIdx, $event)"
                    @contextmenu.prevent="openContextMenu($event, 'memo_item', undefined, undefined, undefined, gIdx, iIdx, item)"
                    :title="'右键菜单操作: ' + item.cmd"
                  >
                    <span class="memo-pillar"></span>
                    <Icons name="doc" size="14" class="memo-icon" />
                    <span class="memo-name">{{ item.name }}</span>
                  </div>
                </div>
              </Transition>
            </div>

            <!-- 底部吸底浅蓝大胶囊按钮 -->
            <div class="memo-bottom-action">
              <button class="btn-bottom-add-pill" @click="openAddMemoModal()">
                <Icons name="plus" size="14" />
                <span>添加命令项</span>
              </button>
            </div>
          </div>
        </aside>

      </div>

      <!-- 视图 2：主机管理大盘视图 (120帧流光条 + 流量统计/校准交互) -->
      <div v-show="currentView === 'mgmt'" class="mgmt-view">
        <div class="mgmt-header">
          <div>
            <h2 class="mgmt-title">综合主机管理</h2>
            <span class="mgmt-sub">支持实时流量审计、结算日重置与莫比乌斯微光轨道展示</span>
          </div>
          <div class="mgmt-actions-row">
            <!-- 每行卡片数选择器 (1-3 列) -->
            <div class="mgmt-cols-picker" title="选择每行显示的卡片列数 (1 - 3 列)">
              <span class="cols-label">排列:</span>
              <button 
                v-for="col in [1, 2, 3]" 
                :key="col"
                :class="['btn-col-choice', mgmtGridColumns === col ? 'active' : '']"
                @click="setMgmtGridColumns(col)"
              >
                {{ col }}列
              </button>
            </div>

            <button class="btn-sec-act" @click="openImportSelectModal">从设备列表导入</button>
            <button class="btn-add-host" @click="openAddHostModal('all')">
              <Icons name="plus" size="14" />
              <span>添加新主机</span>
            </button>
          </div>
        </div>

        <div class="cards-grid" :class="'cols-' + mgmtGridColumns">
          <div 
            v-for="(h, idx) in configData.managed_hosts.filter(item => !searchKeyword || item.name.toLowerCase().includes(searchKeyword.toLowerCase()) || item.host.includes(searchKeyword))" 
            :key="h.host + h.port" 
            :id="`host-card-${h.host}-${h.port}`"
            :class="[
              'host-card', 
              highlightedHostKey === `${h.host}:${h.port}` ? 'breathing-highlight' : '',
              draggingHostIndex === idx ? 'is-dragging' : '',
              dragOverHostIndex === idx && draggingHostIndex !== idx ? 'is-drag-over' : ''
            ]"
            draggable="true"
            @dragstart="handleCardDragStart(idx, $event)"
            @dragover="handleCardDragOver(idx, $event)"
            @dragleave="handleCardDragLeave(idx)"
            @drop="handleCardDrop(idx, $event)"
            @dragend="handleCardDragEnd"
          >
            <div class="card-pillar"></div>
            <div class="card-body">
              <div class="card-row1">
                <span class="card-name">{{ h.name }}</span>
                <span 
                  class="card-latency" 
                  :class="{
                    'is-tcp': (h.latency_text || '').startsWith('TCP'),
                    'is-icmp': (h.latency_text || '').startsWith('ICMP'),
                    'is-err': (h.latency_text || '').includes('超时') || (h.latency_text || '').includes('拒绝') || (h.latency_text || '').includes('失败')
                  }"
                >
                  {{ h.latency_text || '待测速' }}
                </span>
              </div>
              <div class="card-addr">{{ h.host }}:{{ h.port }}</div>

              <div class="card-meta">
                <span>备注: {{ h.details?.memo || '无' }}</span>
                <span>到期: {{ h.details?.expire_date || '无限' }}</span>
                <span>结算日: 每月 {{ h.details?.reset_day || 1 }} 日 {{ String(h.details?.reset_hour || 0).padStart(2, '0') }}:{{ String(h.details?.reset_min || 0).padStart(2, '0') }}:{{ String(h.details?.reset_sec || 0).padStart(2, '0') }}</span>
                <span>价格: {{ h.details?.price || '0.00' }} {{ h.details?.currency || 'CNY' }}</span>
              </div>

              <!-- 流量展示行与右上角独立的"统计详情"交互标签 -->
              <div class="traffic-section">
                <div class="traffic-label">
                  <span class="traffic-text-main">流量: {{ h.details?.traffic_used || '0' }}{{ h.details?.traffic_used_unit || 'GB' }} / {{ h.details?.traffic_total_infinite ? '无限 (∞)' : `${h.details?.traffic_total || '0'}${h.details?.traffic_total_unit || 'GB'}` }}</span>
                  <button 
                    type="button" 
                    class="traffic-audit-btn" 
                    @click.stop="openTrafficStatsModal(h)" 
                    title="点击打开流量详细统计与校准窗口"
                  >
                    <span class="audit-icon">⚙</span>
                    <span class="audit-text">统计详情</span>
                  </button>
                </div>

                <!-- 无限流量：优雅呼吸流光条 -->
                <div v-if="h.details?.traffic_total_infinite" class="unified-traffic-bar infinite">
                  <div class="infinite-flow"></div>
                  <div class="infinity-badge">∞ 无限流量</div>
                </div>
                <!-- 有限流量：真实百分比流光进度条 -->
                <div v-else class="unified-traffic-bar">
                  <div 
                    class="unified-traffic-fill" 
                    :class="{ 'warn': getTrafficProgress(h) >= 80, 'danger': getTrafficProgress(h) >= 95 }"
                    :style="{ width: getTrafficProgress(h) + '%', minWidth: getTrafficProgress(h) > 0 ? '8px' : '0' }"
                  ></div>
                  <span class="unified-traffic-pct">{{ getTrafficProgress(h).toFixed(1) }}%</span>
                </div>
              </div>

              <div class="card-actions">
                <div class="card-actions-left">
                  <button class="btn-card-act btn-primary btn-ssh" @click="openSession(h)">打开 SSH</button>
                  <button class="btn-card-act btn-tcp" @click="testLatency(h, 'tcp')">TCP</button>
                  <button class="btn-card-act btn-icmp" @click="testLatency(h, 'icmp')">ICMP</button>
                  <button class="btn-card-act btn-reset" @click="resetTrafficCounter(h)" title="重置已用流量为 0">清零</button>
                  <button class="btn-card-act btn-edit" @click="openEditHostModal(h, 'asset')">编辑</button>
                </div>
                <div class="card-actions-right">
                  <button class="btn-card-act btn-danger btn-del" @click="confirmDeleteHost(h)" title="删除此主机">删除</button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 告警与事件提醒下拉卡片 (NotificationMonitorPopover 1:1 对标 ssh8.py) -->
    <Transition name="popover-fade">
    <div 
      v-if="showAlertPopover" 
      class="transfer-monitor-popover alert-popover" 
      :style="{ '--arrow-right': (alertPopoverTargetRight - 8) + 'px' }"
    >
      <div class="transfer-popover-header alert-head">
        <span class="alert-head-title">告警与事件提醒</span>
        <div class="alert-head-actions">
          <button 
            v-if="hostAlertList.length > 0" 
            class="btn-popover-ignore" 
            @click="clearAllAlerts"
          >
            全部忽略
          </button>
          <span class="popover-close" @click="showAlertPopover = false">✕</span>
        </div>
      </div>
      <div class="transfer-task-list alert-scroll-list">
        <div v-if="hostAlertList.length === 0" class="alert-empty-state">
          当前所有主机状态良好，无待处理告警
        </div>
        <div 
          v-else 
          v-for="(al, idx) in hostAlertList" 
          :key="idx" 
          class="alert-card-item"
          @click="handleAlertClick(al)"
          title="点击跳转至该主机详情"
        >
          <div class="alert-item-dot-col">
            <span :class="['alert-indicator-dot', al.type]"></span>
          </div>
          <div class="alert-item-content">
            <div class="alert-item-msg">
              <span class="alert-item-hname">[{{ al.hostName }}]</span>
              {{ al.msg }}
            </div>
          </div>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 悬浮传输监视器 (SFTPTransferMonitorPopover) -->
    <Transition name="popover-fade">
    <div 
      v-if="showTransferMonitor" 
      class="transfer-monitor-popover" 
      :style="{ '--arrow-right': (transferPopoverTargetRight - 8) + 'px' }"
    >
      <div class="transfer-popover-header">
        <span>SFTP 传输监视队列 ({{ activeTransfers.length }})</span>
        <div class="popover-header-actions">
          <button 
            class="btn-popover-clear" 
            :disabled="activeTransfers.length === 0"
            @click="clearTransferHistory"
            title="清除传输记录"
          >
            清除历史
          </button>
          <span class="popover-close" @click="showTransferMonitor = false">✕</span>
        </div>
      </div>
      <div class="transfer-task-list">
        <div v-if="activeTransfers.length === 0" class="no-task">暂无传输任务</div>
        <div v-for="t in activeTransfers" :key="t.task_id" class="transfer-task-item">
          <div class="task-info">
            <span class="task-badge" :class="t.is_upload ? 'up' : 'down'">{{ t.is_upload ? '↑ 上传' : '↓ 下载' }}</span>
            <span class="task-name">{{ t.filename }}</span>
            <span class="task-speed">{{ formatSpeed(t.speed_bps) }}</span>
          </div>
          <div class="task-progress-bar">
            <div 
              class="task-progress-fill" 
              :style="{ width: Math.min(100, (t.transferred / (t.total || 1)) * 100) + '%' }"
            ></div>
          </div>
          <div class="task-status-row">
            <span>{{ Math.round((t.transferred / (t.total || 1)) * 100) }}%</span>
            <span class="task-status-actions">
              <span :class="['status-tag', t.status]">{{ t.status === 'completed' ? '完成' : t.status === 'failed' ? '失败' : t.status === 'cancelled' ? '已取消' : '传输中' }}</span>
              <button v-if="t.status === 'running'" class="btn-task-cancel" @click="cancelTransfer(t.task_id)" title="取消传输">取消</button>
            </span>
          </div>
          <div v-if="t.error" class="task-error" :title="t.error">{{ t.error }}</div>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 全局 Material You 拟态原生右键菜单 -->
    <Transition name="context-fade">
    <div 
      v-if="contextMenu.show" 
      ref="contextMenuRef"
      class="context-menu"
      :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }"
    >
      <!-- 设备树主机右键选项 (轻量纯净：连接、编辑、复制IP、删除) -->
      <template v-if="contextMenu.targetType === 'host'">
        <div class="menu-item" @click="openSession(contextMenu.targetHost!)">连接此主机</div>
        <div class="menu-item" @click="openEditHostModal(contextMenu.targetHost!, 'connection')">编辑主机配置</div>
        <div class="menu-item" @click="copyText(contextMenu.targetHost!.host)">复制主机 IP 地址</div>
        <div class="menu-divider"></div>
        <div class="menu-item danger" @click="deleteHost(contextMenu.targetHost!)">删除主机</div>
      </template>

      <!-- 设备树分组与空白处右键选项 -->
      <template v-else-if="contextMenu.targetType === 'group'">
        <div v-if="contextMenu.targetGroup" class="menu-item" @click="openAddHostModal">在此组新建主机</div>
        <div v-else class="menu-item" @click="openAddHostModal">新建主机</div>
        <div v-if="contextMenu.targetGroup" class="menu-item" @click="openRenameGroupModal(contextMenu.targetGroup)">重命名分组</div>
        <div class="menu-item" @click="openNewGroupModal">新建分组</div>
        <div v-if="contextMenu.targetGroup" class="menu-divider"></div>
        <div v-if="contextMenu.targetGroup" class="menu-item danger" @click="deleteGroup(contextMenu.targetGroup)">删除分组</div>
      </template>

      <!-- 备忘录条目右键选项 -->
      <template v-else-if="contextMenu.targetType === 'memo_item'">
        <div
          class="menu-item"
          :class="{ disabled: !activeSession?.alive || activeSession?.host?.protocol === 'telnet' || activeSession?.host?.protocol === 'serial' }"
          @click="fillMemoCommandToTerminal(contextMenu.targetMemoItem!.cmd)"
        >复制到终端</div>
        <div class="menu-item" @click="sendMemoCommand(contextMenu.targetMemoItem!.cmd); closeContextMenu()">立即执行此命令</div>
        <div class="menu-item" @click="openEditMemoModal(contextMenu.targetMemoGroupIdx!, contextMenu.targetMemoItemIdx!, contextMenu.targetMemoItem!)">编辑此命令</div>
        <div class="menu-item" @click="copyText(contextMenu.targetMemoItem!.cmd)">复制命令文本</div>
        <div class="menu-divider"></div>
        <div class="menu-item danger" @click="deleteMemo(contextMenu.targetMemoGroupIdx!, contextMenu.targetMemoItemIdx!)">删除命令</div>
      </template>

      <!-- 备忘录分组与空白处右键选项 -->
      <template v-else-if="contextMenu.targetType === 'memo_group' || contextMenu.targetType === 'memo_bg'">
        <div v-if="contextMenu.targetType === 'memo_group' && contextMenu.targetMemoGroupIdx !== null" class="menu-item" @click="openAddMemoModal(contextMenu.targetMemoGroupIdx)">在此组新建命令</div>
        <div v-else class="menu-item" @click="openAddMemoModal()">新建命令</div>
        <div v-if="contextMenu.targetType === 'memo_group' && contextMenu.targetMemoGroupIdx !== null" class="menu-item" @click="openRenameMemoGroupModal(contextMenu.targetMemoGroupIdx)">重命名分组</div>
        <div class="menu-item" @click="openNewMemoGroupModal">新建命令分组</div>
        <div v-if="contextMenu.targetType === 'memo_group' && contextMenu.targetMemoGroupIdx !== null" class="menu-divider"></div>
        <div v-if="contextMenu.targetType === 'memo_group' && contextMenu.targetMemoGroupIdx !== null" class="menu-item danger" @click="deleteMemoGroup(contextMenu.targetMemoGroupIdx)">删除分组</div>
      </template>

      <!-- 终端标签页右键菜单 (1:1 复刻 ssh8.py CustomTabItemWidget.show_context_menu) -->
      <template v-else-if="contextMenu.targetType === 'tab'">
        <div class="menu-item" @click="contextMenu.targetTab && toggleTabLock(contextMenu.targetTab); closeContextMenu()">
          {{ contextMenu.targetTab?.is_locked ? '解锁标签页' : '锁定标签页' }}
        </div>
        <div class="menu-item" @click="clearCurrentTerminal">清屏终端</div>
        <div class="menu-divider"></div>
        <div 
          :class="['menu-item', contextMenu.targetTab?.is_locked ? 'disabled' : 'danger']" 
          @click="!contextMenu.targetTab?.is_locked && contextMenu.targetTab && (closeSession(contextMenu.targetTab), closeContextMenu())"
        >
          关闭标签页
        </div>
      </template>

      <!-- 终端内容区右键选项 -->
      <template v-else-if="contextMenu.targetType === 'terminal'">
        <div class="menu-item" @click="toggleTerminalReadOnly">
          {{ activeSession?.read_only ? '关闭只读模式' : '开启只读模式（防误操作）' }}
        </div>
        <div class="menu-divider"></div>
        <div class="menu-item" @click="copySelection">复制选中 (Ctrl+Shift+C)</div>
        <div class="menu-item" @click="pasteClipboard">粘贴内容 (Ctrl+Shift+V)</div>
        <div class="menu-divider"></div>
        <div class="menu-item" @click="selectAllTerminal">全选终端 (Select All)</div>
        <div class="menu-item" @click="clearCurrentTerminal">清空屏幕 (Clear)</div>
      </template>

      <!-- SFTP 文件条目右键选项 -->
      <template v-else-if="contextMenu.targetType === 'sftp_item'">
        <div v-if="!contextMenu.targetSftpItem?.is_dir" class="menu-item" @click="sftpTriggerDownload(contextMenu.targetSftpItem!)">下载文件到本地</div>
        <div class="menu-item" @click="sftpOpenRename(contextMenu.targetSftpItem!)">重命名</div>
        <div class="menu-item" @click="sftpOpenChmod(contextMenu.targetSftpItem!)">修改权限 (chmod)</div>
        <div class="menu-item" @click="sftpViewProperties(contextMenu.targetSftpItem!)">查看文件属性</div>
        <div class="menu-divider"></div>
        <div class="menu-item" @click="sftpRefresh">刷新目录</div>
        <div class="menu-divider"></div>
        <div class="menu-item danger" @click="sftpDeleteItem(contextMenu.targetSftpItem!)">
          <template v-if="selectedSftpFiles.length > 1 && selectedSftpFiles.some(f => f.name === contextMenu.targetSftpItem?.name)">
            批量删除 ({{ selectedSftpFiles.length }} 项)
          </template>
          <template v-else>
            删除{{ contextMenu.targetSftpItem?.is_dir ? '文件夹' : '文件' }}
          </template>
        </div>
      </template>

      <!-- SFTP 空白区右键选项 -->
      <template v-else-if="contextMenu.targetType === 'sftp_bg'">
        <template v-if="sftpPanelState === 'ready'">
          <div class="menu-item" @click="sftpTriggerUpload">上传文件到此目录</div>
          <div class="menu-item" @click="sftpOpenNewFolder">新建文件夹</div>
          <div class="menu-divider"></div>
        </template>
        <div v-else class="menu-item disabled">{{ sftpPanelStateText }}</div>
        <div
          class="menu-item"
          :class="{ disabled: sftpPanelState !== 'ready' }"
          @click="sftpPanelState === 'ready' && sftpRefresh()"
        >刷新目录</div>
      </template>
    </div>
    </Transition>

    <!-- 应用内文件选择器：替代 Windows 原生文件框 -->
    <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showLocalFilePicker" class="modal-overlay local-picker-overlay" @mousedown.self="cancelLocalFileBrowser">
      <div class="modal-card local-picker-modal">
        <div class="modal-header">
          <span class="modal-title">{{ localPickerTitle }}</span>
          <span class="modal-close" @click="cancelLocalFileBrowser">✕</span>
        </div>
        <div class="local-picker-body">
          <aside class="local-picker-sidebar">
            <button
              v-for="folder in localKnownFolders"
              :key="folder.id"
              type="button"
              class="local-picker-side-item"
              :title="folder.path"
              @click="openLocalKnownFolder(folder)"
            >
              <Icons :name="folder.icon as any" size="16" />
              <span>{{ folder.name }}</span>
            </button>
            <div class="local-picker-side-divider"></div>
            <button
              type="button"
              :class="['local-picker-side-item', localPickerMode === 'drives' ? 'active' : '']"
              @click="showLocalThisPc"
            >
              <Icons name="host" size="16" />
              <span>此电脑</span>
            </button>
            <button
              type="button"
              :class="['local-picker-side-item', localPickerMode === 'network' ? 'active' : '']"
              @click="showLocalNetwork"
            >
              <Icons name="cloud" size="16" />
              <span>网络</span>
            </button>
          </aside>

          <div class="local-picker-main">
            <div class="local-picker-toolbar">
              <button class="btn-sec-act" @click="localPickerParent">上一级</button>
              <button class="btn-sec-act" @click="localPickerMode === 'drives' ? showLocalThisPc() : loadLocalDir(localPickerPath)">刷新</button>
              <input
                v-model="localPickerPath"
                class="local-picker-path"
                spellcheck="false"
                @keyup.enter="loadLocalDir(localPickerPath)"
              />
            </div>
            <div v-if="localPickerMode === 'files'" class="local-picker-roots">
              <button
                v-for="root in localPickerRoots"
                :key="root.path"
                class="local-picker-root"
                @click="loadLocalDir(root.path)"
              >{{ root.name }}</button>
            </div>
            <div v-if="localPickerError" class="local-picker-error">{{ localPickerError }}</div>
            <div class="local-picker-list">
              <template v-if="localPickerMode === 'drives'">
                <div
                  v-for="root in localPickerRoots"
                  :key="root.path"
                  class="local-picker-item is-dir"
                  @click="loadLocalDir(root.path)"
                >
                  <Icons name="host" size="18" />
                  <span class="local-picker-name">{{ root.name }}</span>
                  <span class="local-picker-size">本地磁盘</span>
                </div>
              </template>
              <div v-else-if="localPickerMode === 'network'" class="local-picker-network-hint">
                <div>输入本地网络路径后按 Enter</div>
                <div class="local-picker-network-example">\\192.168.1.10\share</div>
              </div>
              <div v-else-if="localPickerLoading" class="local-picker-empty">读取目录中...</div>
              <div v-else-if="localPickerEntries.length === 0" class="local-picker-empty">目录为空</div>
              <template v-else>
                <div
                  v-for="entry in localPickerEntries"
                  :key="entry.path"
                  :class="['local-picker-item', entry.is_dir ? 'is-dir' : 'is-file', localPickerSelected?.path === entry.path ? 'selected' : '']"
                  @click="localPickerChoose(entry)"
                  @dblclick="entry.is_dir ? loadLocalDir(entry.path) : confirmLocalFilePicker()"
                >
                  <img
                    v-if="!entry.is_dir && localPickerThumbnails[entry.path]"
                    :src="localPickerThumbnails[entry.path]"
                    :alt="entry.name"
                    class="local-picker-thumb"
                    loading="lazy"
                  />
                  <Icons v-else :name="entry.is_dir ? 'folder' : 'file'" size="18" />
                  <span class="local-picker-name" :title="entry.name">{{ entry.name }}</span>
                  <span v-if="!entry.is_dir" class="local-picker-size">{{ formatMetricBytes(entry.size) }}</span>
                </div>
              </template>
            </div>
            <div v-if="localPickerAction === 'save'" class="local-picker-save-row">
              <label>文件名</label>
              <input
                v-model="localPickerFileName"
                class="local-picker-path"
                spellcheck="false"
                placeholder="输入保存文件名"
                @keyup.enter="confirmLocalFilePicker"
              />
            </div>
          </div>
        </div>
        <div class="modal-footer local-picker-footer">
          <span class="local-picker-selected" :title="localPickerSelected?.path || ''">
            <template v-if="localPickerAction === 'save'">
              {{ localPickerMode === 'files' ? (joinLocalPath(localPickerPath, localPickerFileName) || '请输入文件名') : '请选择保存目录' }}
            </template>
            <template v-else>
              {{ localPickerSelected ? localPickerSelected.name : '请选择文件' }}
            </template>
          </span>
          <button class="btn-cancel" @click="cancelLocalFileBrowser">取消</button>
          <button
            class="btn-confirm"
            :disabled="localPickerAction === 'save'
              ? (localPickerMode !== 'files' || !localPickerFileName.trim())
              : (!localPickerSelected || localPickerSelected.is_dir)"
            @click="confirmLocalFilePicker"
          >{{ localPickerAction === 'save' ? '保存到此处' : '选择文件' }}</button>
        </div>
      </div>
    </div>
    </Transition>
    </Teleport>

    <!-- SSH 连接密码弹窗：主机未保存密码时，在终端连接过程中弹出 -->
    <Transition name="modal-fade">
      <div v-if="sshPasswordModal.show" class="modal-overlay" @mousedown.self="cancelSshPassword">
        <div class="modal-card modal-small ssh-password-modal">
          <div class="modal-header">
            <span class="modal-title">需要 SSH 密码</span>
            <span class="modal-close" @click="cancelSshPassword">✕</span>
          </div>
          <div class="modal-body">
            <div class="ssh-password-host">
              <span class="ssh-password-host-name">{{ sshPasswordModal.hostName }}</span>
              <span class="ssh-password-host-addr">{{ sshPasswordModal.hostLabel }}</span>
            </div>
            <div class="form-row">
              <label>密码</label>
              <div class="password-field-wrap">
                <input
                  ref="sshPasswordInput"
                  v-model="sshPasswordModal.password"
                  type="password"
                  class="modal-input"
                  autocomplete="current-password"
                  placeholder="输入 SSH 登录密码"
                  @keyup.enter="submitSshPassword"
                  @keyup.esc="cancelSshPassword"
                />
              </div>
              <div v-if="sshPasswordModal.error" class="ssh-password-error">{{ sshPasswordModal.error }}</div>
            </div>
            <label class="ssh-remember-row">
              <input v-model="sshPasswordModal.remember" type="checkbox" />
              <span>记住密码</span>
            </label>
            <p class="ssh-password-hint">勾选后会保存到本机配置文件；不勾选则仅用于本次连接。</p>
          </div>
          <div class="modal-footer">
            <button class="btn-cancel" @click="cancelSshPassword">取消</button>
            <button class="btn-confirm" @click="submitSshPassword">连接</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 弹窗 1：主机配置编辑模态弹窗 (结构化优雅排版) -->
    <Transition name="modal-fade">
    <div v-if="hostModal.show" class="modal-overlay" @mousedown.self="hostModal.show = false">
      <div class="modal-card modal-host-edit">
        <div class="modal-header">
          <span class="modal-title">
            {{ hostModal.mode === 'asset' ? '编辑资产与配额' : (hostModal.mode === 'connection' ? (hostModal.isEdit ? '编辑主机配置' : '新建远程主机') : (hostModal.isEdit ? '编辑主机配置' : '新建远程主机')) }}
          </span>
          <span class="modal-close" @click="hostModal.show = false">✕</span>
        </div>

        <div class="modal-body modal-scrollable">
          <!-- 区块 1：基本连接信息 (仅在 connection 或 all 模式展示) -->
          <div v-if="hostModal.mode === 'connection' || hostModal.mode === 'all'" class="form-section-card">
            <div class="modal-section-title">
              <span class="section-title-icon">●</span>
              <span>连接与认证</span>
            </div>
            
            <div class="form-row">
              <label>显示名称</label>
              <input v-model="hostModal.form.name" class="modal-input" placeholder="例如：东京-SoftBank" />
            </div>

                        <!-- 连接协议选择下拉框 (SSH / Telnet / Serial) -->
            <div class="form-row">
              <label>连接协议</label>
              <CustomSelect 
                :modelValue="hostModal.form.protocol || 'ssh'" 
                :options="protocolOptions" 
                customClass="w-full"
                @change="onProtocolChange"
              />
            </div>

            <!-- Telnet 明文安全性警告横幅 (WinUI3 风格 InfoBar) -->
            <div v-if="hostModal.form.protocol === 'telnet'" class="infobar-warning">
              <span class="infobar-icon">⚠️</span>
              <span class="infobar-text">Telnet 为明文未加密协议，登录凭证与会话传输存在被窃听风险，建议仅在可信局域网或受控调试设备中使用。</span>
            </div>

            <!-- 串口高级参数折叠配置 (数据位 / 校验位 / 停止位 / 流控 / 换行符) -->
            <div v-if="hostModal.form.protocol === 'serial'" class="serial-advanced-grid">
              <div class="field-col">
                <label>数据位</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_data_bits || 8"
                  :options="[
                    { label: '8 位 (默认常用)', value: 8 },
                    { label: '7 位', value: 7 },
                    { label: '6 位', value: 6 },
                    { label: '5 位', value: 5 }
                  ]"
                  customClass="w-full"
                  @change="(val: number) => hostModal.form.serial_data_bits = val"
                />
              </div>
              <div class="field-col">
                <label>校验位</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_parity || 'none'"
                  :options="[
                    { label: 'None 无校验 (N)', value: 'none' },
                    { label: 'Even 偶校验 (E)', value: 'even' },
                    { label: 'Odd 奇校验 (O)', value: 'odd' }
                  ]"
                  customClass="w-full"
                  @change="(val: 'none' | 'odd' | 'even') => hostModal.form.serial_parity = val"
                />
              </div>
              <div class="field-col">
                <label>停止位</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_stop_bits || 1"
                  :options="[
                    { label: '1 位 (标准)', value: 1 },
                    { label: '2 位', value: 2 }
                  ]"
                  customClass="w-full"
                  @change="(val: number) => hostModal.form.serial_stop_bits = val"
                />
              </div>
              <div class="field-col">
                <label>流控协议</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_flow_control || 'none'"
                  :options="[
                    { label: 'None 无流控', value: 'none' },
                    { label: 'RTS/CTS 硬件流控', value: 'hardware' },
                    { label: 'XON/XOFF 软件流控', value: 'software' }
                  ]"
                  customClass="w-full"
                  @change="(val: 'none' | 'software' | 'hardware') => hostModal.form.serial_flow_control = val"
                />
              </div>
              <div class="field-col">
                <label>Enter 换行模式</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_newline || 'cr'"
                  :options="[
                    { label: 'CR (\\r 回车)', value: 'cr' },
                    { label: 'CRLF (\\r\\n 回车换行)', value: 'crlf' },
                    { label: 'LF (\\n 换行)', value: 'lf' }
                  ]"
                  customClass="w-full"
                  @change="(val: 'cr' | 'lf' | 'crlf') => hostModal.form.serial_newline = val"
                />
              </div>
              <div class="field-col">
                <label>本地回显 (Local Echo)</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_local_echo ? 'true' : 'false'"
                  :options="[
                    { label: '关 (设备自身回显)', value: 'false' },
                    { label: '开 (本端键入即显)', value: 'true' }
                  ]"
                  customClass="w-full"
                  @change="(val: string) => hostModal.form.serial_local_echo = (val === 'true')"
                />
              </div>
              <div class="field-col">
                <label>DTR 控制信号</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_dtr ? 'true' : 'false'"
                  :options="[
                    { label: 'True 高电平 (使能)', value: 'true' },
                    { label: 'False 低电平 (断开)', value: 'false' }
                  ]"
                  customClass="w-full"
                  @change="(val: string) => hostModal.form.serial_dtr = (val === 'true')"
                />
              </div>
              <div class="field-col">
                <label>RTS 请求发送</label>
                <CustomSelect 
                  :modelValue="hostModal.form.serial_rts ? 'true' : 'false'"
                  :options="[
                    { label: 'True 高电平 (使能)', value: 'true' },
                    { label: 'False 低电平 (断开)', value: 'false' }
                  ]"
                  customClass="w-full"
                  @change="(val: string) => hostModal.form.serial_rts = (val === 'true')"
                />
              </div>
            </div>

            <!-- 本地终端：Shell 类型与说明 -->
            <div v-if="hostModal.form.protocol === 'local'" class="form-row">
              <label>本地 Shell</label>
              <CustomSelect
                :modelValue="hostModal.form.local_shell || 'powershell'"
                :options="[{ label: 'PowerShell', value: 'powershell' }, { label: 'CMD (命令提示符)', value: 'cmd' }]"
                customClass="w-full"
                @change="(val: string) => hostModal.form.local_shell = val as 'powershell' | 'cmd'"
              />
            </div>
            <div v-if="hostModal.form.protocol === 'local'" class="infobar-warning">
              <span class="infobar-icon">ℹ️</span>
              <span class="infobar-text">本地终端直接在本机运行 PowerShell / CMD，不需要地址、端口、用户名或密钥。</span>
            </div>

            <!-- 连接地址与端口/波特率 (根据协议联动；本地终端不需要) -->
            <div v-if="hostModal.form.protocol !== 'local'" :class="['form-row', 'grid-host-port', hostModal.form.protocol === 'serial' ? 'is-serial' : '']">
                            <div class="field-col">
                <label>{{ hostModal.form.protocol === 'serial' ? '串口端口 (COM / 设备名)' : '连接地址 (IP / 域名)' }}</label>
                <!-- Serial 串口模式：始终为纯正下拉选框，同时右侧搭配一键扫描刷新按钮 -->
                <div v-if="hostModal.form.protocol === 'serial'" class="input-combined-row">
                  <CustomSelect 
                    :modelValue="hostModal.form.host || 'COM1'"
                    :options="serialPortOptions"
                    customClass="flex-1"
                    @change="(val: string) => hostModal.form.host = val"
                  />
                  <button 
                    type="button" 
                    :class="['btn-sec-act', 'btn-browse', isScanningPorts ? 'btn-scanning' : '']" 
                    title="重新扫描并刷新本机可用串口"
                    @click="refreshSerialPorts"
                  >
                    {{ isScanningPorts ? '扫描中...' : '刷新' }}
                  </button>
                </div>
                <input 
                  v-else 
                  v-model="hostModal.form.host" 
                  class="modal-input" 
                  placeholder="192.168.1.1 或 domain.com" 
                />
              </div>

              <div class="field-col">
                <label>{{ hostModal.form.protocol === 'serial' ? '波特率' : (hostModal.form.protocol === 'telnet' ? 'Telnet 端口' : 'SSH 端口') }}</label>
                <!-- 串口波特率：一体化复合框（框体内直接键入任意数值，右侧内嵌箭头点击弹出预设下拉菜单） -->
                <div v-if="hostModal.form.protocol === 'serial'" class="combo-baud-wrapper" ref="comboBaudRef">
                  <input 
                    v-model.number="hostModal.form.port" 
                    type="number" 
                    class="modal-input text-center combo-baud-input" 
                    placeholder="9600" 
                  />
                  <button 
                    type="button" 
                    class="combo-baud-arrow" 
                    :class="{ rotated: showBaudDropdown }"
                    title="选择常用波特率预设"
                    @click.stop="toggleBaudDropdown"
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
                      <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  </button>
                  <transition name="select-popover-anim">
                    <div v-if="showBaudDropdown" class="combo-baud-dropdown" >
                      <div 
                        v-for="opt in serialBaudRateOptions" 
                        :key="opt.value" 
                        :class="['combo-baud-option', { active: hostModal.form.port === opt.value }]"
                        @click.stop="selectBaudRate(opt.value)"
                      >
                        <span>{{ opt.label }}</span>
                        <span v-if="hostModal.form.port === opt.value" class="opt-check">✓</span>
                      </div>
                    </div>
                  </transition>
                </div>
                <input 
                  v-else 
                  v-model.number="hostModal.form.port" 
                  type="number" 
                  class="modal-input text-center" 
                />
              </div>
            </div>

            <div v-if="hostModal.form.protocol !== 'local'" class="form-row grid-2">
              <div class="field-col">
                <label>登录用户名</label>
                <input v-model="hostModal.form.user" class="modal-input" placeholder="root" />
              </div>
              <div class="field-col">
                <label>登录密码 (可选)</label>
                <div class="password-field-wrap">
                  <input 
                    v-model="hostModal.form.pwd" 
                    :type="showHostPassword ? 'text' : 'password'" 
                    class="modal-input" 
                    placeholder="密码（或私钥密码）" 
                  />
                  <button 
                    type="button" 
                    class="password-toggle-btn" 
                    :title="showHostPassword ? '隐藏密码' : '显示密码'"
                    @click="showHostPassword = !showHostPassword"
                  >
                    <svg v-if="showHostPassword" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
                      <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
                      <path d="M1 1l22 22" />
                    </svg>
                    <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7S1 12 1 12z" />
                      <circle cx="12" cy="12" r="3" />
                    </svg>
                  </button>
                </div>
              </div>
            </div>

            <div v-if="hostModal.form.protocol !== 'local'" class="form-row">
              <label>私钥证书 (支持 .pem / id_rsa)</label>
              <div class="traffic-input-box">
                <input v-model="hostModal.form.key_path" class="modal-input flex-1" placeholder="留空则使用密码认证" />
                <button class="btn-sec-act btn-browse" type="button" @click="browseKeyFile">浏览选择...</button>
                <button class="btn-sec-act btn-browse" type="button" @click="openKeyGenModal">生成...</button>
              </div>
            </div>

            <!-- 终端类型（SSH / Telnet 适用；串口不涉及 TERM） -->
            <div v-if="hostModal.form.protocol === 'ssh' || hostModal.form.protocol === 'telnet'" class="form-row">
              <label>终端类型 (TERM)</label>
              <CustomSelect
                :modelValue="hostModal.form.term_type || 'auto'"
                :options="termTypeOptions"
                customClass="w-full"
                @change="(val: string) => hostModal.form.term_type = val"
              />
            </div>

            <!-- 使用系统 SSH Agent 认证 -->
            <div v-if="hostModal.form.protocol === 'ssh'" class="form-row">
              <label>认证方式</label>
              <label class="agent-checkbox-row">
                <input type="checkbox" v-model="hostModal.form.use_agent" class="md3-check-native" />
                <span>使用系统 SSH Agent（ssh-agent / Pageant）中的密钥</span>
              </label>
            </div>
          </div>

          <!-- 区块 2：服务器资产与计费配额 (仅在 asset 或 all 模式展示) -->
          <div v-if="hostModal.mode === 'asset' || hostModal.mode === 'all'" class="form-section-card">
            <div class="modal-section-title">
              <span class="section-title-icon">●</span>
              <span>资产与计费配额</span>
            </div>

            <!-- 行 1：机器价格与到期时间 (精确到分) -->
            <div class="form-row grid-2">
              <div class="field-col">
                <label>机器价格与货币</label>
                <div class="input-combined-row">
                  <input v-model="hostModal.form.price" class="modal-input flex-1" placeholder="0.00" />
                  <CustomSelect 
                    v-model="hostModal.form.currency" 
                    :options="[
                      { label: 'CNY (￥)', value: 'CNY' },
                      { label: 'USD ($)', value: 'USD' },
                      { label: 'EUR (€)', value: 'EUR' },
                      { label: 'HKD (HK$)', value: 'HKD' },
                      { label: 'JPY (JP¥)', value: 'JPY' },
                      { label: 'GBP (£)', value: 'GBP' }
                    ]"
                    customClass="w-28 text-center"
                  />
                </div>
              </div>
              <div class="field-col">
                <label>到期时间 (精确到秒)</label>
                <div class="input-combined-row">
                  <CustomDateTimePicker 
                    v-model="hostModal.form.expire_date" 
                    :disabled="hostModal.form.expire_infinite"
                    :show-seconds="true"
                    customClass="flex-1"
                  />
                  <label class="checkbox-label infinite-addon-check" title="永久无限期">
                    <input type="checkbox" v-model="hostModal.form.expire_infinite" />
                    无限期
                  </label>
                </div>
              </div>
            </div>

            <!-- 行 2：总计流量配额 + 已用流量 (基准已用) -->
            <div class="form-row grid-2">
              <div class="field-col">
                <label>总计流量配额</label>
                <div class="input-combined-row">
                  <input 
                    v-model="hostModal.form.traffic_total" 
                    :disabled="hostModal.form.traffic_total_infinite"
                    class="modal-input flex-1" 
                    placeholder="例如 1000"
                  />
                  <CustomSelect 
                    v-model="hostModal.form.traffic_total_unit"
                    :disabled="hostModal.form.traffic_total_infinite"
                    :options="[{ label: 'MB', value: 'MB' }, { label: 'GB', value: 'GB' }, { label: 'TB', value: 'TB' }]"
                    customClass="w-20 text-center"
                  />
                  <label class="checkbox-label infinite-addon-check" title="无限流量模式">
                    <input type="checkbox" v-model="hostModal.form.traffic_total_infinite" />
                    无限
                  </label>
                </div>
              </div>
              <div class="field-col">
                <label>已用流量 (基准已用)</label>
                <div class="input-combined-row">
                  <input 
                    v-model="hostModal.form.traffic_used" 
                    class="modal-input flex-1" 
                    placeholder="0"
                  />
                  <CustomSelect 
                    v-model="hostModal.form.traffic_used_unit"
                    :options="[{ label: 'MB', value: 'MB' }, { label: 'GB', value: 'GB' }, { label: 'TB', value: 'TB' }]"
                    customClass="w-24 text-center"
                  />
                </div>
              </div>
            </div>

            <!-- 行 3：流量重置时间 (每月几号 + 精确到秒) -->
            <div class="form-row">
              <label>流量重置时间 (每月结算重置时刻，精确到秒)</label>
              <div class="reset-time-row">
                <span class="time-prefix">每月</span>
                <CustomNumberStepper 
                  v-model="hostModal.form.reset_day" 
                  :min="1" 
                  :max="31" 
                  customClass="reset-day-stepper"
                />
                <span class="time-suffix">号</span>
                <CustomNumberStepper 
                  v-model="hostModal.form.reset_hour" 
                  :min="0" 
                  :max="23" 
                  :padZero="true" 
                  customClass="reset-digit-stepper"
                />
                <span class="time-colon">:</span>
                <CustomNumberStepper 
                  v-model="hostModal.form.reset_min" 
                  :min="0" 
                  :max="59" 
                  :padZero="true" 
                  customClass="reset-digit-stepper"
                />
                <span class="time-colon">:</span>
                <CustomNumberStepper 
                  v-model="hostModal.form.reset_sec" 
                  :min="0" 
                  :max="59" 
                  :padZero="true" 
                  customClass="reset-digit-stepper"
                />
                <span class="time-hint">(时 : 分 : 秒)</span>
              </div>
            </div>

            <!-- 行 4：流量检测网卡名称 + 流量统计方向 -->
            <div class="form-row grid-2">
              <div class="field-col">
                <label>流量检测网卡名称</label>
                <input 
                  v-model="hostModal.form.target_interface" 
                  class="modal-input" 
                  placeholder="留空自动（默认路由 / eth0）"
                />
                <div class="field-hint">
                  支持多个网卡：用逗号分隔，如 <code>eth0,eth1</code>；填 <code>*</code> 统计所有物理网卡；支持 <code>eth*</code> 通配。多个网卡会累加；桥接/虚拟网卡请勿和物理网卡同时填写，避免重复计数。
                </div>
              </div>
              <div class="field-col">
                <label>流量统计方向</label>
                <CustomSelect 
                  v-model="hostModal.form.traffic_direction" 
                  :options="[
                    { label: '双向流量统计 (Tx + Rx)', value: 'both' },
                    { label: '仅计出站流量 (Tx)', value: 'tx' },
                    { label: '仅计入站流量 (Rx)', value: 'rx' }
                  ]"
                  customClass="w-full"
                />
              </div>
            </div>

            <!-- 行 5：流量价格 (超额/流量单价) -->
            <div class="form-row">
              <label>流量价格 (每 GB 价格 / 流量包费用)</label>
              <div class="input-combined-row">
                <input 
                  v-model="hostModal.form.traffic_price" 
                  class="modal-input flex-1" 
                  placeholder="0.00" 
                />
                <CustomSelect 
                  v-model="hostModal.form.traffic_currency" 
                  :options="[
                    { label: 'CNY (￥)', value: 'CNY' },
                    { label: 'USD ($)', value: 'USD' },
                    { label: 'EUR (€)', value: 'EUR' },
                    { label: 'HKD (HK$)', value: 'HKD' },
                    { label: 'JPY (JP¥)', value: 'JPY' },
                    { label: 'GBP (£)', value: 'GBP' }
                  ]"
                  customClass="w-28 text-center"
                />
              </div>
            </div>
          </div>

          <!-- 区块 3：专属备注 (仅在 asset 或 all 模式展示) -->
          <div v-if="hostModal.mode === 'asset' || hostModal.mode === 'all'" class="form-section-card">
            <div class="modal-section-title">
              <span class="section-title-icon">●</span>
              <span>备注说明</span>
            </div>
            <div class="form-row">
              <label>专属备注说明</label>
              <input v-model="hostModal.form.memo" class="modal-input" placeholder="机器用途、配置规格或购买商家等" />
            </div>
          </div>
        </div>

        <div class="modal-footer">
          <button class="btn-cancel" @click="hostModal.show = false">取消</button>
          <button class="btn-confirm" @click="saveHostForm">保存配置</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 2：流量统计与校准 (TrafficStatsDialog) -->
    <Transition name="modal-fade">
    <div v-if="trafficStatsModal.show" class="modal-overlay" @mousedown.self="trafficStatsModal.show = false">
      <div class="modal-card">
        <div class="modal-header">
          <span class="modal-title">流量统计 · {{ trafficStatsModal.host?.name }}</span>
          <span class="modal-close" @click="trafficStatsModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <div class="form-row">
            <label>统计方式:</label>
            <CustomSelect 
              v-model="trafficStatsModal.source" 
              :options="[
                { label: '自动 (vnstat → 本地计数器)', value: 'auto' },
                { label: 'vnstat', value: 'vnstat' },
                { label: '本地计数器 (/proc/net/dev)', value: 'proc' },
                { label: '自定义命令', value: 'custom' }
              ]"
              customClass="w-full"
            />
          </div>
          <div v-if="trafficStatsModal.source === 'custom'" class="form-row">
            <label>自定义审计命令:</label>
            <input v-model="trafficStatsModal.customCommand" class="modal-input" placeholder="输出示例：123.45 GB 或 123456789" />
          </div>
          <div class="form-row">
            <label>当前已用流量校准:</label>
            <div class="traffic-input-box">
              <input 
                v-model="trafficStatsModal.calibUsed" 
                class="modal-input flex-1" 
                @input="trafficStatsModal.manualOverride = true"
                @focus="trafficStatsModal.calibEditing = true"
                @blur="trafficStatsModal.calibEditing = false"
              />
              <CustomSelect 
                v-model="trafficStatsModal.calibUnit" 
                :options="[{ label: 'MB', value: 'MB' }, { label: 'GB', value: 'GB' }, { label: 'TB', value: 'TB' }]"
                customClass="w-24 text-center"
              />
            </div>
            <label class="md3-checkbox-label traffic-manual-check">
              <input type="checkbox" v-model="trafficStatsModal.manualOverride" class="md3-check-native" />
              <span>手动基准 + 自动增量（自动审计不覆盖手工基准）</span>
            </label>
            <div v-if="trafficStatsModal.manualOverride" class="field-hint">
              手工值作为周期基准，自动审计会在此基础上累加增量；取消勾选后完全由自动统计接管。
            </div>
          </div>
          <div class="form-row">
            <label>每月重置结算日:</label>
            <CustomNumberStepper 
              v-model="trafficStatsModal.resetDay" 
              :min="1" 
              :max="31" 
              customClass="w-32"
            />
          </div>
          <div class="traffic-sync-row">
            <button 
              class="traffic-sync-btn" 
              :disabled="trafficStatsModal.isSyncing" 
              @click="syncTrafficStatsNow"
            >
              <span v-if="trafficStatsModal.isSyncing" class="traffic-sync-spinner"></span>
              {{ trafficStatsModal.isSyncing ? '正在获取服务器信息...' : '立即获取服务器信息' }}
            </button>
          </div>
          <div 
            v-if="trafficStatsModal.syncMessage" 
            :class="['traffic-sync-status', trafficStatsModal.syncError ? 'is-error' : '']"
          >
            {{ trafficStatsModal.syncMessage }}
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="trafficStatsModal.show = false">取消</button>
          <button class="btn-confirm" @click="saveTrafficStats">保存并生效</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 3：SFTP 文件属性 (SFTPPropertiesDialog) -->
    <Transition name="modal-fade">
    <div v-if="sftpPropsModal.show" class="modal-overlay" @mousedown.self="sftpPropsModal.show = false">
      <div class="modal-card">
        <div class="modal-header">
          <span class="modal-title">文件属性</span>
          <span class="modal-close" @click="sftpPropsModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <div class="prop-row"><span class="k">名称:</span><span class="v">{{ sftpPropsModal.name }}</span></div>
          <div class="prop-row"><span class="k">完整路径:</span><span class="v">{{ sftpPropsModal.full_path }}</span></div>
          <div class="prop-row"><span class="k">类型:</span><span class="v">{{ sftpPropsModal.is_dir ? '目录' : '常规文件' }}</span></div>
          <div class="prop-row"><span class="k">大小:</span><span class="v">{{ sftpPropsModal.size_bytes }} 字节</span></div>
          <div class="prop-row"><span class="k">权限:</span><span class="v">{{ sftpPropsModal.mode_str }} ({{ sftpPropsModal.mode_oct }})</span></div>
          <div class="prop-row"><span class="k">修改时间:</span><span class="v">{{ sftpPropsModal.mtime_str }}</span></div>
        </div>
        <div class="modal-footer">
          <button class="btn-confirm" @click="sftpPropsModal.show = false">确定</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 4：SFTP 修改权限 (SFTPChmodDialog) -->
    <Transition name="modal-fade">
    <div v-if="sftpChmodModal.show" class="modal-overlay" @mousedown.self="sftpChmodModal.show = false">
      <div class="modal-card">
        <div class="modal-header">
          <span class="modal-title">修改权限 (chmod) · {{ sftpChmodModal.fileName }}</span>
          <span class="modal-close" @click="sftpChmodModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <table class="chmod-table">
            <thead>
              <tr>
                <th>身份</th>
                <th>读取 (r)</th>
                <th>写入 (w)</th>
                <th>执行 (x)</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>所有者 (Owner)</td>
                <td><input type="checkbox" v-model="sftpChmodModal.u_r" /></td>
                <td><input type="checkbox" v-model="sftpChmodModal.u_w" /></td>
                <td><input type="checkbox" v-model="sftpChmodModal.u_x" /></td>
              </tr>
              <tr>
                <td>用户组 (Group)</td>
                <td><input type="checkbox" v-model="sftpChmodModal.g_r" /></td>
                <td><input type="checkbox" v-model="sftpChmodModal.g_w" /></td>
                <td><input type="checkbox" v-model="sftpChmodModal.g_x" /></td>
              </tr>
              <tr>
                <td>其他人 (Other)</td>
                <td><input type="checkbox" v-model="sftpChmodModal.o_r" /></td>
                <td><input type="checkbox" v-model="sftpChmodModal.o_w" /></td>
                <td><input type="checkbox" v-model="sftpChmodModal.o_x" /></td>
              </tr>
            </tbody>
          </table>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="sftpChmodModal.show = false">取消</button>
          <button class="btn-confirm" @click="sftpSaveChmod">应用权限</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 5：通用 Prompt 弹窗 (新建文件夹/重命名) -->
    <Transition name="modal-fade">
    <div v-if="promptModal.show" class="modal-overlay" @mousedown.self="promptModal.show = false">
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">{{ promptModal.title }}</span>
          <span class="modal-close" @click="promptModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <div class="form-row">
            <label>{{ promptModal.label }}</label>
            <input v-model="promptModal.value" class="modal-input" @keyup.enter="executePromptModal" autofocus />
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="promptModal.show = false">取消</button>
          <button class="btn-confirm" @click="executePromptModal">确定</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 5.5：通用优雅确认弹窗 (替换系统丑陋的默认 confirm 弹窗) -->
    <Transition name="modal-fade">
    <div v-if="customConfirmModal.show" class="modal-overlay" @mousedown.self="customConfirmModal.show = false">
      <div class="modal-card modal-small modal-confirm-card">
        <div class="modal-header">
          <span class="modal-title">{{ customConfirmModal.title }}</span>
          <span class="modal-close" @click="customConfirmModal.show = false">✕</span>
        </div>
        <div class="modal-body confirm-modal-body">
          <p class="confirm-modal-text">{{ customConfirmModal.message }}</p>
          <div v-if="customConfirmModal.items && customConfirmModal.items.length" class="confirm-modal-items-list">
            <div v-for="(it, idx) in customConfirmModal.items" :key="idx" class="confirm-modal-item-row">
              <span class="confirm-item-bullet">•</span>
              <span class="confirm-item-name">{{ it }}</span>
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-confirm" @click="customConfirmModal.show = false">取消</button>
          <button 
            :class="customConfirmModal.danger ? 'btn-danger' : 'btn-cancel'" 
            @click="customConfirmModal.onConfirm"
          >
            {{ customConfirmModal.confirmText }}
          </button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 配置已加密：启动解锁 -->
    <UnlockModal
      v-model="unlockModalShow"
      :busy="unlockBusy"
      :error="unlockError"
      @submit="submitUnlock"
    />

    <!-- 开启 / 关闭配置加密 -->
    <EncryptionModal
      v-model="encryptionModalShow"
      :mode="encryptionModalMode"
      :busy="encryptionModalBusy"
      :error="encryptionModalError"
      @submit="submitEncryptionChange"
    />

    <!-- SSH 密钥生成向导 -->
    <KeyGenModal
      v-model="keyGenModalShow"
      :default-comment="keyGenDefaultComment"
      @generated="onKeyGenerated"
    />

    <!-- 传输文件冲突：覆盖 / 重命名 / 取消 -->
    <Transition name="modal-fade">
      <div v-if="transferConflictModal.show" class="modal-overlay transfer-conflict-overlay" @mousedown.self="chooseTransferConflict('cancel')">
        <div class="modal-card modal-small">
          <div class="modal-header">
            <span class="modal-title">{{ transferConflictModal.title }}</span>
            <span class="modal-close" @click="chooseTransferConflict('cancel')">✕</span>
          </div>
          <div class="modal-body confirm-modal-body">
            <p class="confirm-modal-text">{{ transferConflictModal.message }}</p>
            <p class="confirm-modal-text" style="margin-top: 8px; opacity: 0.8;">
              {{ transferConflictModal.fileName }}
            </p>
          </div>
          <div class="modal-footer">
            <button class="btn-cancel" @click="chooseTransferConflict('cancel')">取消</button>
            <button class="btn-sec-act" @click="chooseTransferConflict('skip')">跳过</button>
            <button class="btn-sec-act" @click="chooseTransferConflict('rename')">重命名</button>
            <button class="btn-danger" @click="chooseTransferConflict('overwrite')">覆盖</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 弹窗 6：备忘录指令编辑弹窗 (MemoEditDialog) -->
    <Transition name="modal-fade">
    <div v-if="memoEditModal.show" class="modal-overlay" @mousedown.self="memoEditModal.show = false">
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">{{ memoEditModal.isEdit ? '编辑指令' : '新建常用指令' }}</span>
          <span class="modal-close" @click="memoEditModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <div class="form-row">
            <label>指令名称:</label>
            <input v-model="memoEditModal.form.name" class="modal-input" placeholder="例如：重启 Nginx" />
          </div>
          <div class="form-row">
            <label>执行命令:</label>
            <textarea v-model="memoEditModal.form.cmd" class="modal-input modal-textarea" placeholder="例如：systemctl restart nginx"></textarea>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="memoEditModal.show = false">取消</button>
          <button class="btn-confirm" @click="saveMemo">保存</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 7：终端偏好与备份设置 (1:1 严格对标 ssh8.py TerminalSettingsDialog) -->
    <Transition name="modal-fade">
    <div v-if="showTerminalSettingsModal" class="modal-overlay" @mousedown.self="showTerminalSettingsModal = false">
      <div class="modal-card settings-modal-card">
        <div class="modal-header">
          <span class="modal-title">终端偏好与备份设置</span>
          <span class="modal-close" @click="showTerminalSettingsModal = false">✕</span>
        </div>
        <div class="modal-body settings-form-body">
          <!-- 0. 全局界面字体 (除 xterm 终端外统一生效) -->
          <div class="settings-form-row">
            <label class="settings-form-label">界面字体:</label>
            <div class="settings-form-field flex-row-gap">
              <span class="selected-font-preview" :title="appFontFamily">{{ getDisplayFontName(appFontFamily) }}</span>
              <button type="button" class="btn-card-act" @click="showFontSelectorModal = true">
                更换字体...
              </button>
            </div>
          </div>

          <!-- 1. 等宽字体 (xterm 终端专用) -->
          <div class="settings-form-row">
            <label class="settings-form-label">终端等宽字体:</label>
            <div class="settings-form-field">
              <CustomSelect 
                v-model="terminalSettings.font_family" 
                :options="[
                  { label: 'Consolas', value: 'Consolas' },
                  { label: 'Cascadia Code', value: 'Cascadia Code' },
                  { label: 'JetBrains Mono', value: 'JetBrains Mono' },
                  { label: 'Courier New', value: 'Courier New' },
                  { label: 'Fira Code', value: 'Fira Code' }
                ]"
                customClass="w-full"
              />
            </div>
          </div>

          <!-- 2. 字号大小 (CustomNumberStepper 步进调节) -->
          <div class="settings-form-row">
            <label class="settings-form-label">字号大小:</label>
            <div class="settings-form-field">
              <CustomNumberStepper 
                v-model="terminalSettings.font_size" 
                :min="8" 
                :max="28" 
                customClass="w-24 text-center"
              />
            </div>
          </div>

          <!-- 3. 字体加粗 -->
          <div class="settings-form-row">
            <label class="settings-form-label">字体加粗:</label>
            <div class="settings-form-field">
              <label class="md3-checkbox-label">
                <input type="checkbox" v-model="terminalSettings.is_bold" class="md3-check-native" />
                <span>加粗 (Bold)</span>
              </label>
            </div>
          </div>

          <!-- 4. 光标形状 -->
          <div class="settings-form-row">
            <label class="settings-form-label">光标形状:</label>
            <div class="settings-form-field">
              <CustomSelect 
                v-model="terminalSettings.cursor_style" 
                :options="[
                  { label: '块状光标', value: 'block', icon: 'block' },
                  { label: '竖线光标', value: 'bar', icon: 'bar' },
                  { label: '下划线光标', value: 'underline', icon: 'underline' }
                ]"
                customClass="w-full"
              />
            </div>
          </div>

          <!-- 5. 光标闪烁与频率 -->
          <div class="settings-form-row">
            <label class="settings-form-label">光标闪烁:</label>
            <div class="settings-form-field blink-row">
              <label class="md3-checkbox-label">
                <input type="checkbox" v-model="terminalSettings.cursor_blink" class="md3-check-native" />
                <span>启用闪烁</span>
              </label>
              <span class="blink-speed-label">频率(ms):</span>
              <CustomNumberStepper 
                v-model="terminalSettings.blink_interval" 
                :step="50" 
                :min="200" 
                :max="2000" 
                :disabled="!terminalSettings.cursor_blink"
                customClass="w-28 text-center"
              />
            </div>
          </div>

          <!-- 6. 终端背景预设 (黑/白) -->
          <div class="settings-form-row">
            <label class="settings-form-label">终端背景预设:</label>
            <div class="settings-form-field bg-preset-row">
              <button 
                type="button" 
                :class="['btn-bg-preset', terminalSettings.term_bg_mode === 'dark' ? 'active' : '']"
                @click="setTerminalBgPreset('dark')"
              >
                黑色背景 (黑底绿字)
              </button>
              <button 
                type="button" 
                :class="['btn-bg-preset', terminalSettings.term_bg_mode === 'light' ? 'active' : '']"
                @click="setTerminalBgPreset('light')"
              >
                白色背景 (浅灰黑字)
              </button>
            </div>
          </div>

          <!-- 6.1 自定义终端背景图 -->
          <div class="settings-form-row">
            <label class="settings-form-label">终端背景图:</label>
            <div class="settings-form-field bg-image-settings">
              <button type="button" class="btn-sec-act btn-browse" @click="chooseTerminalBackgroundImage">
                选择图片
              </button>
              <button
                v-if="terminalSettings.custom_bg_image"
                type="button"
                class="btn-cancel"
                @click="removeTerminalBackgroundImage"
              >移除背景图</button>
              <span v-if="terminalSettings.custom_bg_image" class="bg-image-path" :title="terminalSettings.custom_bg_image">
                {{ terminalSettings.custom_bg_image }}
              </span>
              <span v-else class="bg-image-hint">支持 PNG / JPG / WEBP / GIF / BMP，最大 25 MB</span>
            </div>
          </div>

          <div v-if="terminalSettings.custom_bg_image" class="settings-form-row">
            <label class="settings-form-label">图片裁切:</label>
            <div class="settings-form-field">
              <CustomSelect
                v-model="terminalSettings.bg_image_fit"
                :options="[
                  { label: '裁切填满（推荐）', value: 'cover' },
                  { label: '完整显示', value: 'contain' },
                  { label: '拉伸铺满', value: 'stretch' },
                  { label: '平铺', value: 'tile' }
                ]"
                customClass="w-full"
                @change="applyTerminalSettings(false)"
              />
            </div>
          </div>

          <div v-if="terminalSettings.custom_bg_image" class="settings-form-row">
            <label class="settings-form-label">背景遮罩:</label>
            <div class="settings-form-field bg-image-dim-row">
              <input
                v-model.number="terminalSettings.bg_image_dim"
                type="range"
                min="0"
                max="0.85"
                step="0.05"
                class="bg-image-dim-slider"
                @input="updateTerminalBackgroundPreview"
                @change="applyTerminalSettings(false)"
              />
              <span class="bg-image-dim-value">{{ Math.round(terminalSettings.bg_image_dim * 100) }}%</span>
            </div>
          </div>

          <div v-if="terminalSettings.custom_bg_image" class="settings-form-row">
            <label class="settings-form-label">文字可读性:</label>
            <div class="settings-form-field">
              <label class="md3-checkbox-label">
                <input
                  v-model="terminalSettings.bg_image_auto_fg"
                  type="checkbox"
                  class="md3-check-native"
                  @change="updateTerminalBackgroundPreview(); applyTerminalSettings(false)"
                />
                <span>根据背景亮度自动调整文字颜色</span>
              </label>
            </div>
          </div>

          <!-- 7. 强调色 (按钮主色) -->
          <div class="settings-form-row">
            <label class="settings-form-label">强调色:</label>
            <div class="settings-form-field accent-picker-row">
              <ColorPicker
                :model-value="terminalSettings.accent_color || '#A8C7FA'"
                :presets="accentPresetColors"
                title="自定义强调色"
                @update:model-value="setAccentColor"
              />
              <button type="button" class="btn-accent-reset" @click="resetAccentColor">跟随主题</button>
            </div>
          </div>

          <!-- 8. 终端文字色彩 (调色块 + 自定义取色盘) -->
          <div class="settings-form-row">
            <label class="settings-form-label">终端文字色彩:</label>
            <div class="settings-form-field color-palette-row">
              <button 
                v-for="c in (terminalSettings.term_bg_mode === 'dark' ? darkPresetColors : lightPresetColors)" 
                :key="c"
                type="button"
                :class="['color-swatch-btn', terminalSettings.terminal_fg_color.toLowerCase() === c.toLowerCase() ? 'active' : '']"
                :style="{ backgroundColor: c }"
                @click="terminalSettings.terminal_fg_color = c"
                :title="c"
              ></button>
              <!-- 自定义取色按钮 -->
              <ColorPicker
                :model-value="terminalSettings.terminal_fg_color"
                :presets="terminalSettings.term_bg_mode === 'dark' ? darkPresetColors : lightPresetColors"
                title="自定义终端文字颜色"
                @update:model-value="terminalSettings.terminal_fg_color = $event"
              />
            </div>
          </div>

          <!-- 8. 开机启动 -->
          <div class="settings-form-row">
            <label class="settings-form-label">开机启动:</label>
            <div class="settings-form-field">
              <label class="md3-checkbox-label">
                <input
                  type="checkbox"
                  :checked="autostartEnabled"
                  class="md3-check-native"
                  @change="handleAutostartChange"
                />
                <span>开机时自动启动 QShell（后台托盘运行）</span>
              </label>
            </div>
          </div>

          <!-- 9. 关闭行为 -->
          <div class="settings-form-row">
            <label class="settings-form-label">关闭窗口:</label>
            <div class="settings-form-field">
              <label class="md3-checkbox-label">
                <input type="checkbox" v-model="terminalSettings.minimize_to_tray" class="md3-check-native" />
                <span>点击关闭按钮时隐藏到系统托盘（不退出程序）</span>
              </label>
            </div>
          </div>

          <!-- 10. 全局动画过渡效果 -->
          <div class="settings-form-row">
            <label class="settings-form-label">视觉动画:</label>
            <div class="settings-form-field">
              <label class="md3-checkbox-label">
                <input 
                  type="checkbox" 
                  :checked="animationsEnabled" 
                  class="md3-check-native" 
                  @change="setAnimationsEnabled(($event.target as HTMLInputElement).checked)"
                />
                <span>启用全局平滑动画过渡与交互微动效</span>
              </label>
            </div>
          </div>

          <!-- 11. 配置加密（主密码） -->
          <div class="settings-form-row">
            <label class="settings-form-label">配置加密:</label>
            <div class="settings-form-field flex-row-gap config-encryption-field">
              <button
                type="button"
                class="btn-sec-act config-encryption-btn"
                @click="openEncryptionModal(configEncryptionEnabled ? 'disable' : 'enable')"
              >
                {{ configEncryptionEnabled ? '关闭配置加密' : '开启配置加密' }}
              </button>
              <span class="config-encryption-hint">
                {{ configEncryptionEnabled ? '已开启：配置以 AES-256-GCM 加密保存' : '未开启：主机密码、私钥口令以明文保存' }}
              </span>
            </div>
          </div>

          <!-- 12. 版本更新 -->
          <div class="settings-form-row">
            <label class="settings-form-label">版本更新:</label>
            <div class="settings-form-field flex-row-gap config-encryption-field">
              <button
                type="button"
                class="btn-sec-act config-encryption-btn"
                :disabled="updating"
                @click="checkForUpdates(false)"
              >
                {{ updating ? '更新中...' : '检查更新' }}
              </button>
              <span class="config-encryption-hint">当前版本 v{{ appVersion }}</span>
            </div>
          </div>

          <!-- 9. 备份相关入口 -->
          <div class="backup-entry-row">
            <button type="button" class="btn-backup-center" @click="openBackupFromSettings">
              备份相关 (本地备份 / WebDAV 云同步)
            </button>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="showTerminalSettingsModal = false">取消</button>
          <button class="btn-confirm" @click="applyTerminalSettings">保存</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 7.5：全局界面字体更换选择弹窗 (FontSelectorModal) -->
    <Transition name="modal-fade">
    <div v-if="showFontSelectorModal" class="modal-overlay" @mousedown.self="showFontSelectorModal = false">
      <div class="modal-card font-selector-modal-card">
        <div class="modal-header">
          <span class="modal-title">更换界面字体 (全局生效)</span>
          <span class="modal-close" @click="showFontSelectorModal = false">✕</span>
        </div>
        <div class="modal-body font-selector-body">
          <div class="font-selector-toolbar">
            <label class="md3-checkbox-label">
              <input 
                type="checkbox" 
                v-model="includeSystemFonts" 
                @change="handleToggleSystemFonts" 
                class="md3-check-native" 
              />
              <span>读取系统字体</span>
            </label>
            <span v-if="isReadingSystemFonts" class="font-loading-tip">正在读取系统中...</span>
          </div>

          <div class="font-list-scrollable">
            <!-- 推荐默认与精选字体 -->
            <div class="font-group-title">精选与默认界面字体</div>
            <div 
              v-for="item in presetUiFonts" 
              :key="item.value"
              :class="['font-item-row', appFontFamily === item.value ? 'active' : '']"
              @click="applyAppFont(item.value)"
            >
              <div class="font-item-left">
                <span class="font-item-name">{{ item.name }}</span>
                <span class="font-item-preview" :style="{ fontFamily: item.value }">QShell Next 快速连接终端 1234567890</span>
              </div>
              <span v-if="appFontFamily === item.value" class="font-check-mark">✓</span>
            </div>

            <!-- 读取到的系统字体 -->
            <template v-if="includeSystemFonts">
              <div class="font-group-title">系统已安装字体 ({{ systemFontList.length }})</div>
              <div 
                v-for="f in systemFontList" 
                :key="f"
                :class="['font-item-row', appFontFamily.includes(f) ? 'active' : '']"
                @click="applyAppFont(`'${f}', sans-serif`)"
              >
                <div class="font-item-left">
                  <span class="font-item-name">{{ f }}</span>
                  <span class="font-item-preview" :style="{ fontFamily: `'${f}', sans-serif` }">QShell Next 终端系统 1234567890 AaBbCc</span>
                </div>
                <span v-if="appFontFamily.includes(f)" class="font-check-mark">✓</span>
              </div>
            </template>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-confirm" @click="showFontSelectorModal = false">完成选择</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 8：备份与云同步管理 (BackupManagementDialog) -->
    <Transition name="modal-fade">
    <div v-if="backupModal.show" class="modal-overlay" @mousedown.self="backupModal.show = false">
      <div class="modal-card">
        <div class="modal-header">
          <span class="modal-title">配置备份与 WebDAV 云同步</span>
          <span class="modal-close" @click="backupModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <div class="backup-section-title">本地配置备份</div>
          <div class="backup-btn-row">
            <button class="btn-sec-act flex-1" @click="handleExportLocal">导出备份到本地 (.zip)</button>
            <button class="btn-sec-act flex-1" @click="handleImportLocal">从本地恢复配置 (.zip)</button>
          </div>

          <div class="backup-divider"></div>
          <div class="backup-section-title">WebDAV 云端同步</div>
          <div class="form-row">
            <label>服务器 URL:</label>
            <input v-model="backupModal.webdav_url" class="modal-input" placeholder="https://dav.example.com/dav/" @input="scheduleWebdavSettingsSave" />
          </div>
          <div class="form-row grid-2">
            <div>
              <label>用户名 / 邮箱:</label>
              <input v-model="backupModal.webdav_user" class="modal-input" @input="scheduleWebdavSettingsSave" />
            </div>
            <div>
              <label>密码 / 应用授权码:</label>
              <div class="password-field-wrap">
                <input 
                  v-model="backupModal.webdav_pwd" 
                  :type="showWebdavPassword ? 'text' : 'password'" 
                  class="modal-input" 
                  @input="scheduleWebdavSettingsSave" 
                />
                <button 
                  type="button" 
                  class="password-toggle-btn" 
                  :title="showWebdavPassword ? '隐藏密码' : '显示密码'"
                  @click="showWebdavPassword = !showWebdavPassword"
                >
                  <svg v-if="showWebdavPassword" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
                    <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
                    <path d="M1 1l22 22" />
                  </svg>
                  <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7S1 12 1 12z" />
                    <circle cx="12" cy="12" r="3" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
          <div class="form-row">
            <label>云端存储文件路径:</label>
            <input v-model="backupModal.webdav_path" class="modal-input" placeholder="/QShell/qshell_backup.zip" @input="scheduleWebdavSettingsSave" />
          </div>

          <div class="backup-btn-row">
            <button class="btn-sec-act" :disabled="backupModal.is_busy" @click="handleWebdavTest">测试连接</button>
            <button class="btn-sec-act" :disabled="backupModal.is_busy" @click="handleWebdavUpload">立即上传备份</button>
            <button class="btn-sec-act" :disabled="backupModal.is_busy" @click="handleWebdavDownload">从云端恢复</button>
          </div>

          <div v-if="backupModal.status_msg" class="backup-status-text">
            {{ backupModal.status_msg }}
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="backupModal.show = false">关闭</button>
        </div>
      </div>
    </div>
    </Transition>

    <!-- 弹窗 9：从设备列表导入主机 (HostImportSelectionDialog) -->
    <Transition name="modal-fade">
    <div v-if="importSelectModal.show" class="modal-overlay" @mousedown.self="importSelectModal.show = false">
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">导入主机到管理面板</span>
          <span class="modal-close" @click="importSelectModal.show = false">✕</span>
        </div>
        <div class="modal-body">
          <div v-if="importSelectModal.candidates.length === 0" class="no-task">
            所有设备树主机均已在主机管理中，无需重复导入。
          </div>
          <div v-else class="import-list">
            <div 
              v-for="(h, idx) in importSelectModal.candidates" 
              :key="h.host + h.port" 
              class="import-item"
            >
              <label class="checkbox-label">
                <input 
                  type="checkbox" 
                  :value="idx" 
                  v-model="importSelectModal.selectedIndexes" 
                />
                <span class="import-hname">{{ h.name }}</span>
                <span class="import-haddr">({{ h.host }}:{{ h.port }})</span>
              </label>
            </div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="importSelectModal.show = false">取消</button>
          <button 
            v-if="importSelectModal.candidates.length > 0" 
            class="btn-confirm" 
            @click="confirmImportSelected"
          >
            确认导入 ({{ importSelectModal.selectedIndexes.length }})
          </button>
        </div>
      </div>
    </div>
    </Transition>

  </div>
</template>

<style src="./styles/app.css"></style>
