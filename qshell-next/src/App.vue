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
const appVersion = ref("0.99.70");
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
const keyKindOptions = [
  { label: "ED25519（推荐）", value: "ed25519" },
  { label: "RSA 4096", value: "rsa4096" },
  { label: "RSA 2048", value: "rsa2048" },
  { label: "ECDSA P-256", value: "ecdsa" },
];

const keyGenModal = ref({
  show: false,
  kind: "ed25519",
  directory: "",
  fileName: "id_ed25519",
  passphrase: "",
  comment: "",
  generating: false,
  error: "",
  result: null as null | {
    private_path: string;
    public_path: string;
    public_key: string;
    fingerprint: string;
    algorithm: string;
  },
});

function openKeyGenModal() {
  const user = hostModal.value.form.user || "root";
  const host = hostModal.value.form.host || "host";
  keyGenModal.value = {
    show: true,
    kind: "ed25519",
    directory: "",
    fileName: "id_ed25519",
    passphrase: "",
    comment: `${user}@${host}`,
    generating: false,
    error: "",
    result: null,
  };
}

function onKeyKindChange(kind: string) {
  keyGenModal.value.kind = kind;
  const defaults: Record<string, string> = {
    ed25519: "id_ed25519",
    rsa4096: "id_rsa4096",
    rsa2048: "id_rsa",
    ecdsa: "id_ecdsa",
  };
  keyGenModal.value.fileName = defaults[kind] || "id_qshell";
}

async function generateSshKey() {
  const m = keyGenModal.value;
  m.generating = true;
  m.error = "";
  try {
    const info = await invoke<any>("generate_ssh_key", {
      kind: m.kind,
      directory: m.directory.trim() || null,
      fileName: m.fileName,
      passphrase: m.passphrase || null,
      comment: m.comment || null,
    });
    m.result = info;
    // 生成成功后直接填入主机配置
    hostModal.value.form.key_path = info.private_path;
    if (m.passphrase) hostModal.value.form.passphrase = m.passphrase;
  } catch (e: any) {
    m.error = String(e?.message || e);
  } finally {
    m.generating = false;
  }
}

async function copyPublicKey() {
  const key = keyGenModal.value.result?.public_key;
  if (key) {
    await writeClipboardText(key);
  }
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
const unlockModal = ref({ show: false, password: "", error: "", busy: false });
const encryptionModal = ref({
  show: false,
  mode: "enable" as "enable" | "disable",
  password: "",
  confirm: "",
  error: "",
  busy: false,
});

async function refreshConfigEncryptionState() {
  if (!isTauri) return;
  try {
    configEncryptionEnabled.value = await invoke<boolean>("is_config_encrypted");
  } catch (e) {
    console.warn("[config] 读取加密状态失败:", e);
  }
}

function openEncryptionModal(mode: "enable" | "disable") {
  encryptionModal.value = {
    show: true,
    mode,
    password: "",
    confirm: "",
    error: "",
    busy: false,
  };
}

async function submitEncryptionChange() {
  const m = encryptionModal.value;
  if (!m.password) {
    m.error = "请输入主密码";
    return;
  }
  if (m.mode === "enable" && m.password !== m.confirm) {
    m.error = "两次输入的密码不一致";
    return;
  }
  if (m.mode === "enable" && m.password.length < 4) {
    m.error = "主密码至少 4 位";
    return;
  }
  m.busy = true;
  m.error = "";
  try {
    if (m.mode === "enable") {
      // 先把当前内存里的最新配置落盘，再整体加密
      await invoke("save_hosts_config", { config: configData.value });
    }
    await invoke("set_config_encryption", { enabled: m.mode === "enable", password: m.password });
    configEncryptionEnabled.value = m.mode === "enable";
    m.show = false;
  } catch (e: any) {
    m.error = String(e?.message || e);
  } finally {
    m.busy = false;
  }
}

async function submitUnlock() {
  const m = unlockModal.value;
  if (!m.password) {
    m.error = "请输入主密码";
    return;
  }
  m.busy = true;
  m.error = "";
  try {
    await invoke("unlock_config", { password: m.password });
    m.show = false;
    m.password = "";
    configLocked.value = false;
    await loadConfig();
  } catch (e: any) {
    m.error = String(e?.message || e);
  } finally {
    m.busy = false;
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
          unlockModal.value.show = true;
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
    <Transition name="modal-fade">
      <div v-if="unlockModal.show" class="modal-overlay" style="z-index: 400000;">
        <div class="modal-card modal-small">
          <div class="modal-header">
            <span class="modal-title">配置已加密</span>
          </div>
          <div class="modal-body">
            <p class="confirm-modal-text">请输入主密码解锁本地配置。忘记密码只能清空配置重来。</p>
            <input
              v-model="unlockModal.password"
              type="password"
              class="modal-input"
              placeholder="主密码"
              @keyup.enter="submitUnlock"
            />
            <div v-if="unlockModal.error" class="infobar-warning" style="margin-top: 8px;">
              <span class="infobar-icon">⚠️</span>
              <span class="infobar-text">{{ unlockModal.error }}</span>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-confirm" :disabled="unlockModal.busy" @click="submitUnlock">
              {{ unlockModal.busy ? '解锁中...' : '解锁' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 开启 / 关闭配置加密 -->
    <Transition name="modal-fade">
      <div v-if="encryptionModal.show" class="modal-overlay" style="z-index: 400000;" @mousedown.self="encryptionModal.show = false">
        <div class="modal-card modal-small">
          <div class="modal-header">
            <span class="modal-title">{{ encryptionModal.mode === 'enable' ? '开启配置加密' : '关闭配置加密' }}</span>
            <span class="modal-close" @click="encryptionModal.show = false">✕</span>
          </div>
          <div class="modal-body">
            <p v-if="encryptionModal.mode === 'enable'" class="confirm-modal-text">
              设置主密码后，本地配置文件将被 AES-256-GCM 加密。忘记密码将无法恢复，只能清空配置重新开始。
            </p>
            <p v-else class="confirm-modal-text">
              输入当前主密码关闭加密，配置文件将恢复为明文保存。
            </p>
            <input
              v-model="encryptionModal.password"
              type="password"
              class="modal-input"
              placeholder="主密码"
              @keyup.enter="submitEncryptionChange"
            />
            <input
              v-if="encryptionModal.mode === 'enable'"
              v-model="encryptionModal.confirm"
              type="password"
              class="modal-input"
              style="margin-top: 8px;"
              placeholder="再次输入主密码"
              @keyup.enter="submitEncryptionChange"
            />
            <div v-if="encryptionModal.error" class="infobar-warning" style="margin-top: 8px;">
              <span class="infobar-icon">⚠️</span>
              <span class="infobar-text">{{ encryptionModal.error }}</span>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-cancel" @click="encryptionModal.show = false">取消</button>
            <button class="btn-confirm" :disabled="encryptionModal.busy" @click="submitEncryptionChange">
              {{ encryptionModal.busy ? '处理中...' : '确定' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- SSH 密钥生成向导 -->
    <Transition name="modal-fade">
      <div v-if="keyGenModal.show" class="modal-overlay" @mousedown.self="keyGenModal.show = false">
        <div class="modal-card modal-small">
          <div class="modal-header">
            <span class="modal-title">生成 SSH 密钥</span>
            <span class="modal-close" @click="keyGenModal.show = false">✕</span>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>密钥类型</label>
              <CustomSelect
                :modelValue="keyGenModal.kind"
                :options="keyKindOptions"
                customClass="w-full"
                @change="onKeyKindChange"
              />
            </div>
            <div class="form-row">
              <label>保存目录</label>
              <input v-model="keyGenModal.directory" class="modal-input" placeholder="留空 = 用户目录下的 .ssh 文件夹" />
            </div>
            <div class="form-row">
              <label>文件名</label>
              <input v-model="keyGenModal.fileName" class="modal-input" />
            </div>
            <div class="form-row">
              <label>口令（可选）</label>
              <input v-model="keyGenModal.passphrase" type="password" class="modal-input" placeholder="留空表示无口令" />
            </div>
            <div class="form-row">
              <label>注释（可选）</label>
              <input v-model="keyGenModal.comment" class="modal-input" />
            </div>
            <div v-if="keyGenModal.error" class="infobar-warning" style="margin-top: 8px;">
              <span class="infobar-icon">⚠️</span>
              <span class="infobar-text">{{ keyGenModal.error }}</span>
            </div>
            <div v-if="keyGenModal.result" class="keygen-result">
              <div class="keygen-line">私钥：{{ keyGenModal.result.private_path }}</div>
              <div class="keygen-line">公钥：{{ keyGenModal.result.public_path }}</div>
              <div class="keygen-line">指纹：{{ keyGenModal.result.fingerprint }}</div>
              <div class="keygen-pubkey">{{ keyGenModal.result.public_key }}</div>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-cancel" @click="keyGenModal.show = false">关闭</button>
            <button v-if="keyGenModal.result" class="btn-sec-act" @click="copyPublicKey">复制公钥</button>
            <button class="btn-confirm" :disabled="keyGenModal.generating" @click="generateSshKey">
              {{ keyGenModal.generating ? '生成中...' : '生成' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

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

<style>
/* 核心暗黑主题 Material You 设计系统 */
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body, html, #app {
  width: 100%;
  height: 100%;
  overflow: hidden;
  font-family: var(--app-font-family);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-rendering: optimizeLegibility;
  background: transparent;
}

button, input, select {
  font-family: var(--app-font-family);
}

/* --- 按钮字重：统一为中等字重，避免全局粗体 --- */
button {
  font-weight: 500 !important;
}

/* 卡片操作、SFTP、列选择等按钮 */
.btn-card-act,
.btn-sftp-action,
.btn-col-choice,
.sftp-btn,
.btn-popover-ignore,
.btn-mini-tag {
  font-size: 11px !important;
  font-weight: 500 !important;
}

/* 药丸按键、弹窗确定/取消、添加主机等 */
.btn-mgmt-pill,
.btn-sec-act,
.btn-add-host,
.btn-cancel,
.btn-confirm,
.btn-backup-center,
.btn-bg-preset {
  font-size: 12px !important;
  font-weight: 500 !important;
}

:root {
  --app-font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", sans-serif;
  /* ===================== Material Design 3 (MD3) 纯中性暗色调色板 (MD3 Neutral - 零偏色、纯正中性灰层次) ===================== */
  /* MD3 Neutral Surface (基底底色): 纯粹无偏色的中性深灰 (N-10/12) */
  --bg-app: #151517;
  --bg-main: #151517;

  /* MD3 Neutral Surface Container (卡片与容器 - Elevation Level 1 & 2): 纯正中性深灰 (N-20/22) */
  --bg-card: #202024;
  --bg-capsule: #2A2A2E;
  --bg-capsule-hover: #35353A;
  --bg-sub-card: #1B1B1E;
  --bg-input: #1B1B1E;
  --bg-track: #35353A;
  --bg-modal: #2A2A2E;

  /* MD3 Neutral Outline (边界线规范): 纯正中性轮廓线 (N-50/60)，边界绝对分明 */
  --border-card: #444449;
  --border-capsule: rgba(255, 255, 255, 0.1);
  --border-input: #444449;

  /* MD3 Neutral On-Surface & On-Surface-Variant (文字规范 - 纯正象牙白与中性灰) */
  --text-main: #E3E3E6;
  --text-title: #F2F2F5;
  --text-sub: #A8A8B0;

  /* MD3 Primary (明朗科技淡蓝) */
  --btn-primary-bg: #A8C7FA;
  --btn-primary-hover: #BFD6FC;
  --btn-primary-active: #8EB7F8;
  --btn-primary-text: #042E61;

  --btn-sec-bg: #2A2A2E;
  --btn-sec-border: #444449;
  --btn-sec-text: #E3E3E6;
  --btn-sec-hover: #35353A;

  --color-primary: var(--btn-primary-bg);
  --color-primary-hover: var(--btn-primary-hover);
  --color-primary-active: var(--btn-primary-active);
  --color-on-primary: var(--btn-primary-text);
  --color-accent-gold: #F8D675;

  /* 顶栏图标与窗口按钮 */
  --nav-icon-color: #A8A8B0;
  --nav-icon-hover-bg: rgba(255, 255, 255, 0.08);
  --nav-icon-hover-color: #F2F2F5;
  --win-btn-hover-bg: rgba(255, 255, 255, 0.09);
  --win-btn-active-bg: rgba(255, 255, 255, 0.16);
  --win-btn-color: #E3E3E6;
  --term-empty-bg: #1B1B1E;

  --radius-btn: 6px;
  --radius-card: 12px;
  --radius-capsule: 8px;

  /* 滚动条滑块配色 */
  --scrollbar-thumb: #43474E;
  --scrollbar-thumb-hover: var(--color-primary);
}

[data-theme="light"] {
  /* ===================== 亮色模式模板 (Warm Neutral Light Theme - 温润护眼、清晰层次) ===================== */
  --bg-app: #ECECEE;
  --bg-main: #ECECEE;
  --bg-card: #FFFFFF;
  --bg-capsule: #E2E2E6;
  --bg-capsule-hover: #D6D6DB;
  --bg-sub-card: #F4F4F6;
  --bg-input: #FFFFFF;
  --bg-track: #D8D8DE;
  --bg-modal: #FFFFFF;

  --border-card: #CDCDD3;
  --border-capsule: rgba(0, 0, 0, 0.08);
  --border-input: #C4C4CA;

  --text-main: #1D1D20;
  --text-title: #111114;
  --text-sub: #5E5E66;

  --btn-primary-bg: #2057A6;
  --btn-primary-hover: #2E69BF;
  --btn-primary-active: #184587;
  --btn-primary-text: #FFFFFF;

  --btn-sec-bg: #E4E4E8;
  --btn-sec-border: #CDCDD3;
  --btn-sec-text: #1D1D20;
  --btn-sec-hover: #D8D8DE;

  --color-primary: var(--btn-primary-bg);
  --color-primary-hover: var(--btn-primary-hover);
  --color-primary-active: var(--btn-primary-active);
  --color-on-primary: var(--btn-primary-text);
  --color-accent-gold: #B45309;

  /* 顶栏图标与窗口按钮 */
  --nav-icon-color: #4B4B52;
  --nav-icon-hover-bg: rgba(0, 0, 0, 0.06);
  --nav-icon-hover-color: #1D1D20;
  --win-btn-hover-bg: rgba(0, 0, 0, 0.07);
  --win-btn-active-bg: rgba(0, 0, 0, 0.12);
  --win-btn-color: #2D2D32;
  --term-empty-bg: #FFFFFF;

  /* 滚动条滑块配色 (日间浅色模式：雅致温润中灰，悬停强调色) */
  --scrollbar-thumb: #B8B8C0;
  --scrollbar-thumb-hover: var(--color-primary);
}

/* =========================================================================
   全局滚动条与终端滚动条 1:1 对标 ssh8.py 设计语言 (QScrollBar & Xterm Scrollbar)
   width: 8px; transparent track; thumb: #43474E; border-radius: 4px; hover: @primary@
   ========================================================================= */
::-webkit-scrollbar,
*::-webkit-scrollbar {
  width: 8px !important;
  height: 8px !important;
  background: transparent !important;
}

::-webkit-scrollbar-track,
*::-webkit-scrollbar-track {
  background: transparent !important;
}

::-webkit-scrollbar-thumb,
*::-webkit-scrollbar-thumb {
  background: var(--scrollbar-thumb) !important;
  border-radius: 4px !important;
  min-height: 24px !important;
}

::-webkit-scrollbar-thumb:hover,
*::-webkit-scrollbar-thumb:hover {
  background: var(--scrollbar-thumb-hover) !important;
}

/* 说明：xterm.js 6 使用自绘滚动条 (SmoothScrollableElement)，
   不再走 ::-webkit-scrollbar，真实样式见文件末尾的
   “终端 xterm.js 6 自绘滚动条” 规则。 */

/* 现代化 View Transitions (圆形涟漪水波扩散) 专用样式 */
::view-transition-old(root),
::view-transition-new(root) {
  animation: none;
  mix-blend-mode: normal;
}
::view-transition-old(root) {
  z-index: 1;
}
::view-transition-new(root) {
  z-index: 999999;
}

html,
body,
#app {
  background: var(--bg-app) !important;
}

/* 全局动画总开关抑制规则：当关闭全局动画时，彻底切断所有过渡与关键帧 */
html.disable-animations *,
html.disable-animations *::before,
html.disable-animations *::after {
  animation-duration: 0.001ms !important;
  animation-iteration-count: 1 !important;
  transition-duration: 0.001ms !important;
}

/* 日夜切换的 View Transition 期间暂停其他装饰动画和颜色过渡，
   避免 LED、阴影、卡片过渡与整页快照同时争抢 GPU 合成。 */
html.theme-switching *,
html.theme-switching *::before,
html.theme-switching *::after {
  transition-duration: 0.001ms !important;
  animation-play-state: paused !important;
}

::view-transition-old(root),
::view-transition-new(root) {
  animation-duration: 700ms;
}

html.window-resizing,
html.window-resizing body,
html.window-resizing #app {
  background: var(--bg-app) !important;
}

/* 窗口拉伸/缩放期间：彻底锁死所有几何形变、位移过渡与阴影过度重绘，确保卡片0延迟紧密贴合外边缘 */
html.window-resizing *,
html.window-resizing *::before,
html.window-resizing *::after {
  transition: none !important;
  animation: none !important;
  box-shadow: none !important;
  text-shadow: none !important;
}

html.window-resizing .right-panel-card,
html.window-resizing .center-terminal-card,
html.window-resizing .left-panel,
html.window-resizing .workspace-layout {
  transform: none !important;
}

.app-root {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  background: var(--bg-app);
  color: var(--text-main);
  user-select: none;
  position: relative;
  overflow: hidden;
}

/* 顶部极简原生拖拽顶栏 (一体化框体顶头) */
.title-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 44px;
  background: transparent;
  padding: 0 14px 0 16px;
  user-select: none;
  flex-shrink: 0;
  z-index: 20;
}

.brand {
  display: flex;
  align-items: center;
  gap: 4px;
}

.logo-title {
  font-family: "Arial Black", Gadget, sans-serif;
  font-size: 19px;
  font-weight: 600;
  color: var(--color-primary);
  letter-spacing: 1px;
  line-height: 1;
}

/* 1:1 对标 ssh8.py tag_col: QVBoxLayout 置顶上浮 8px (往上移动2px) */
.tag-version-wrapper {
  display: flex;
  flex-direction: column;
  justify-content: flex-start;
  margin-bottom: 8px;
  margin-left: 2px;
  transform: translateY(-2px);
}

.version-badge {
  display: inline-flex;
  align-items: center;
  gap: 2.5px;
  height: 13px;
  padding: 0 4px;
  border-radius: 6.5px;
  background: rgba(46, 36, 18, 0.86);
  border: 1px solid #E5C158;
}

.version-badge .badge-dot {
  font-size: 5px;
  color: #E5C158;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  transform: translateY(0.5px);
}

.version-badge .badge-text {
  font-family: -apple-system, BlinkMacSystemFont, Segoe UI, Roboto, sans-serif;
  font-size: 8.5px;
  font-weight: 500;
  color: #FFE082;
  letter-spacing: 0.1px;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  transform: translateY(0.5px);
}

[data-theme="light"] .version-badge {
  background: rgba(254, 243, 214, 0.9);
  border: 1px solid #D9A404;
}

[data-theme="light"] .version-badge .badge-dot {
  color: #B45309;
}

[data-theme="light"] .version-badge .badge-text {
  color: #78350F;
}

.top-right-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.btn-mgmt-pill {
  height: 28px;
  padding: 0 16px;
  font-size: 12px;
  font-weight: 500;
  color: var(--color-on-primary);
  background: var(--color-primary);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-btn);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
}

.btn-mgmt-pill:hover {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
}

.btn-mgmt-pill:active {
  background: var(--color-primary-active);
  transform: translateY(1px);
}

.btn-mgmt-pill.active {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: var(--color-on-primary);
}

/* 1:1 对标 ssh8.py WindowControlButton: 固定尺寸 34px × 30px，平铺排布 */
.top-icon-btn {
  width: 34px;
  height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: 6px;
  color: var(--nav-icon-color);
  cursor: pointer;
  position: relative;
  transition: all 0.16s cubic-bezier(0.4, 0, 0.2, 1);
}

.top-icon-btn:hover {
  background: var(--nav-icon-hover-bg);
  color: var(--color-primary);
}

[data-theme="light"] .top-icon-btn:hover {
  background: var(--nav-icon-hover-bg);
  color: var(--color-primary);
}

.top-icon-btn.active {
  color: var(--color-primary) !important;
  background: rgba(120, 217, 236, 0.12) !important;
}

[data-theme="light"] .top-icon-btn.active {
  color: var(--color-primary) !important;
  background: rgba(0, 0, 0, 0.07) !important;
}

/* =========================================================================
   1:1 对标 ssh8.py 顶栏图标互动动画 (Micro-Interactions)
   - 切换主题: 360° OutBack 弹性平滑旋转
   - 铃铛通知: 挂环支点物理级钟摆摇摆 (Swing)
   - 传输任务: 上传往上冲、下载往下沉交错冲刺 (Arrows Shift)
   - 偏好设置: 机械齿轮 60° 咬合正转 (Gear Rotate)
   - 侧边栏: 折叠指示箭头弹性横移 (Collapse Shift)
   ========================================================================= */

/* 1. 主题切换图标动效 */
.top-icon-btn:hover .icon-moon,
.top-icon-btn:hover .icon-sun,
.top-icon-btn:active .icon-moon,
.top-icon-btn:active .icon-sun {
  animation: theme-spin 0.48s cubic-bezier(0.34, 1.56, 0.64, 1);
}
@keyframes theme-spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}

/* 2. 铃铛摇晃动效 (以顶部挂环 cx, cy-7 为支点) */
.top-icon-btn:hover .icon-bell,
.top-icon-btn.has-alert:hover .icon-bell {
  transform-origin: 12px 5px;
  animation: bell-swing 0.4s ease-in-out;
}
@keyframes bell-swing {
  0% { transform: rotate(0deg); }
  25% { transform: rotate(16deg); }
  50% { transform: rotate(-14deg); }
  75% { transform: rotate(8deg); }
  100% { transform: rotate(0deg); }
}

/* 3. 传输双向箭头交互动效 (左箭头上冲，右箭头下沉) */
.top-icon-btn:hover .icon-transfer .arrow-up-group {
  animation: arrow-up-dash 0.35s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.top-icon-btn:hover .icon-transfer .arrow-down-group {
  animation: arrow-down-dash 0.35s cubic-bezier(0.34, 1.56, 0.64, 1);
}
@keyframes arrow-up-dash {
  0% { transform: translateY(0); }
  50% { transform: translateY(-3px); }
  100% { transform: translateY(0); }
}
@keyframes arrow-down-dash {
  0% { transform: translateY(0); }
  50% { transform: translateY(3px); }
  100% { transform: translateY(0); }
}

/* 4. 偏好设置：六齿齿轮外圈原位自转，中心轴心稳固不动，杜绝横移 */
.top-icon-btn:hover .icon-settings .gear-outer-teeth {
  transform-origin: 12px 12px;
  animation: gear-spin 0.5s cubic-bezier(0.34, 1.56, 0.64, 1);
}
@keyframes gear-spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(60deg); }
}

/* 5. 侧边栏折叠横向横移微动效 */
.top-icon-btn:hover .icon-collapse .collapse-arrow {
  animation: collapse-arrow-nudge 0.3s ease-out;
}
@keyframes collapse-arrow-nudge {
  0% { transform: translateX(0); }
  50% { transform: translateX(2.5px); }
  100% { transform: translateX(0); }
}

.alert-dot {
  position: absolute;
  top: 5px;
  right: 6px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #EF4444;
  box-shadow: 0 0 5px rgba(239, 68, 68, 0.8);
}

.window-controls {
  display: flex;
  align-items: center;
  margin-left: 2px;
}

/* 1:1 对标 ssh8.py WindowControlButton (34px × 30px) */
.win-btn {
  width: 34px;
  height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: 6px;
  color: var(--win-btn-color);
  cursor: pointer;
  transition: background 0.16s, color 0.16s;
}

.win-btn:hover,
#snap-btn.is-hovered {
  background: rgba(255, 255, 255, 0.09);
  color: #FFFFFF;
}

[data-theme="light"] .win-btn:hover,
[data-theme="light"] #snap-btn.is-hovered {
  background: rgba(0, 0, 0, 0.08);
  color: #0F172A;
}

.win-btn.close:hover {
  background: #BA1A1A !important;
  color: #FFFFFF !important;
}

/* 主工作区布局 (一体化大框体核心内容区) */
.main-body {
  flex: 1;
  display: flex;
  overflow: hidden;
  position: relative;
  background: transparent;
  padding: 10px;
  min-width: 0;
}

.workspace-layout {
  display: flex;
  width: 100%;
  height: 100%;
  position: relative;
  min-width: 0;
  transition: opacity 0.22s cubic-bezier(0.16, 1, 0.3, 1), transform 0.22s cubic-bezier(0.16, 1, 0.3, 1);
  transform: translateZ(0);
}

/* 隐藏时保持原位与几何尺寸不变，杜绝 xterm 字符度量与行宽归零 */
.workspace-layout.view-hidden {
  position: absolute;
  top: 10px;
  left: 10px;
  right: 10px;
  bottom: 10px;
  opacity: 0;
  transform: translateY(-8px) translateZ(0);
  pointer-events: none;
  z-index: 0;
}

/* 分割拖拽手柄 */
.splitter-handle {
  width: 8px;
  margin: 0 -2px;
  cursor: col-resize;
  position: relative;
  z-index: 10;
  background: transparent !important;
}

.splitter-handle-v {
  height: 8px;
  margin: -2px 0;
  cursor: row-resize;
  position: relative;
  z-index: 10;
  background: transparent !important;
}

/* 通用圆角独立卡片设计 */
.card-container {
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  overflow: hidden;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4), 0 4px 16px rgba(0, 0, 0, 0.35);
  transition: background-color 0.25s ease, border-color 0.25s ease, box-shadow 0.25s ease;
}

[data-theme="light"] .card-container {
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05), 0 4px 16px rgba(0, 0, 0, 0.06);
}

.card-header-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-title);
  padding: 12px 14px 6px 14px;
}

/* 左栏：监控卡片 + 设备列表卡片 (原版宽幅) */
.left-panel {
  display: flex;
  flex-direction: column;
  background: transparent;
  border: none;
  flex-shrink: 0;
  height: 100%;
  overflow: hidden;
}

.monitor-card {
  padding: 0 14px 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 7px;
  box-sizing: border-box;
}

.gauge-group {
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.gauge-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.gauge-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--text-main);
  font-weight: 500;
}

.gauge-track {
  position: relative;
  height: 6px;
  background: var(--bg-track);
  border: 1px solid var(--border-card);
  border-radius: 2px;
  overflow: visible;
  box-shadow: none;
}

.gauge-fill {
  position: absolute;
  left: 0;
  top: 50%;
  height: 10px;
  border-radius: 2px;
  overflow: hidden;
  transform: translateY(-50%);
  transition: width 0.4s cubic-bezier(0.22, 1, 0.36, 1);
}

.gauge-fill.cpu { 
  background: var(--metric-cpu-to);
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.28);
}
.gauge-fill.mem { 
  background: var(--metric-mem-to);
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.28);
}
.gauge-fill.disk { 
  background: var(--metric-disk-to);
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.28);
}

[data-theme="light"] .gauge-fill.cpu,
[data-theme="light"] .gauge-fill.mem,
[data-theme="light"] .gauge-fill.disk {
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.14);
}

.metric-detail {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  min-height: 15px;
  margin-top: 4px;
  color: var(--text-main);
  font-size: 10px;
  font-weight: 500;
  line-height: 1.2;
}

.metric-model {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-family: Consolas, monospace;
  font-size: 10px;
  color: var(--text-main);
}

.metric-chip {
  flex: 0 0 auto;
  height: 16px;
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  border-radius: 3px;
  border: 1px solid var(--border-capsule);
  background: var(--bg-capsule);
  color: var(--text-main);
  font-size: 9px;
  font-weight: 600;
}

/* 网络接口状态胶囊 (仿版本号 Tag 质感，优雅圆角矩形，深邃极客微光) */
.net-if-status-row {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  margin-top: 2px;
  margin-bottom: 2px;
}

.net-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 3px;
  margin-bottom: 1px;
}

.net-section-title {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-sub);
  letter-spacing: 0.3px;
}

.net-if-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 22px;
  padding: 0 8px;
  border-radius: 5px;
  background: rgba(16, 37, 53, 0.82);
  border: 1px solid rgba(120, 217, 236, 0.45);
  box-shadow: none;
  transition: all 0.25s ease;
  cursor: pointer;
  font-family: inherit;
}

.net-if-wrap {
  position: relative;
}

.if-chevron {
  width: 0;
  height: 0;
  border-left: 3px solid transparent;
  border-right: 3px solid transparent;
  border-top: 4px solid #78D9EC;
  font-size: 0;
  line-height: 0;
  flex: 0 0 auto;
  margin-left: 1px;
}

.net-if-menu {
  position: absolute;
  right: 0;
  bottom: calc(100% + 4px);
  width: 164px;
  max-height: 180px;
  overflow-y: auto;
  padding: 4px;
  background: var(--bg-modal);
  border: 1px solid var(--border-card);
  border-radius: 7px;
  box-shadow: 0 8px 22px rgba(0, 0, 0, 0.32);
  z-index: 50;
}

.net-if-option {
  width: 100%;
  height: 26px;
  padding: 0 6px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text-main);
  font-family: inherit;
  font-size: 10px;
  font-weight: 500;
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  text-align: left;
}

.net-if-option:hover {
  background: var(--bg-capsule-hover);
}

.net-if-option.active {
  color: var(--color-primary);
}

.if-option-dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0.55;
  flex: 0 0 auto;
}

.if-option-check {
  margin-left: auto;
  font-size: 10px;
}

.net-if-empty {
  padding: 8px;
  color: var(--text-sub);
  font-size: 10px;
  text-align: center;
}

.net-if-badge .if-dot {
  font-size: 8px;
  color: #78D9EC;
  line-height: 1;
}

.net-if-badge .if-text {
  font-family: -apple-system, BlinkMacSystemFont, Segoe UI, Roboto, monospace, sans-serif;
  font-size: 10px;
  font-weight: 500;
  color: #BEEBF7;
  letter-spacing: 0.2px;
  line-height: 1;
}

/* 亮色模式适配 (清爽科技天蓝圆角矩形) */
[data-theme="light"] .net-if-badge {
  background: #E0F2FE;
  border: 1px solid #7DD3FC;
  box-shadow: 0 1px 2px rgba(2, 132, 199, 0.1);
}

[data-theme="light"] .net-if-badge .if-dot {
  color: #0284C7;
}

[data-theme="light"] .net-if-badge .if-text {
  color: #0369A1;
}

/* 未连接/闲置状态微调 */
.net-if-badge.idle {
  background: rgba(30, 34, 43, 0.6);
  border-color: rgba(255, 255, 255, 0.1);
}
.net-if-badge.idle .if-dot {
  color: #94A3B8;
}
.net-if-badge.idle .if-text {
  color: #94A3B8;
}
[data-theme="light"] .net-if-badge.idle {
  background: #F1F5F9;
  border-color: #CBD5E1;
}
[data-theme="light"] .net-if-badge.idle .if-dot,
[data-theme="light"] .net-if-badge.idle .if-text {
  color: #64748B;
}

.net-panel {
  width: 100%;
  height: 112px;
  margin-top: 4px;
  padding: 6px;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: 8px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.net-live-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: row;
  gap: 6px;
}

.net-live-view .net-sub-card {
  flex: 1;
  min-width: 0;
  min-height: 0;
  padding: 0 2px;
  gap: 2px;
}

.net-live-view .net-sub-card + .net-sub-card {
  border-left: 1px solid var(--border-capsule);
  border-top: 0;
  padding-left: 6px;
  padding-top: 0;
}

.net-live-view .led-svg-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
}

.net-live-view .led-matrix-svg {
  width: 100%;
  height: 100%;
}

.net-view-toggle {
  height: 20px;
  padding: 0 7px;
  margin-left: auto;
  margin-right: 4px;
  border: 1px solid var(--border-input);
  border-radius: 5px;
  background: var(--bg-capsule);
  color: var(--text-sub);
  font-family: inherit;
  font-size: 10px;
  cursor: pointer;
}

.net-view-toggle:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
}

.net-stats-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 0 2px;
}

.net-stats-legend {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  padding: 0 2px 2px;
  border-bottom: 1px solid var(--border-capsule);
  font-size: 11px;
  font-family: Consolas, monospace;
}

.net-stats-chart {
  flex: 1;
  min-height: 0;
  display: flex;
}

.net-chart-svg {
  width: 100%;
  height: 100%;
}

.chart-axis {
  stroke: var(--border-capsule);
  stroke-width: 1;
}

.chart-grid {
  stroke: var(--border-capsule);
  stroke-width: 0.5;
  opacity: 0.55;
}

.chart-tick {
  fill: var(--text-main);
  font-size: 9px;
  font-family: Consolas, monospace;
}

.chart-line-up {
  fill: none;
  stroke: #78D9EC;
  stroke-width: 1.2;
  stroke-linejoin: round;
  stroke-linecap: round;
}

.chart-line-down {
  fill: none;
  stroke: #81C995;
  stroke-width: 1.2;
  stroke-linejoin: round;
  stroke-linecap: round;
}

.net-stats-up {
  color: #78D9EC;
  white-space: nowrap;
}

.net-stats-down {
  color: #81C995;
  white-space: nowrap;
}

.net-traffic-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0;
  margin-top: 4px;
  padding: 5px 0 3px;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: 8px;
  overflow: hidden;
}

.net-sub-card {
  background: transparent;
  border: 0;
  border-radius: 0;
  padding: 0 4px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  transition: background-color 0.2s, border-color 0.2s;
}

.net-sub-card + .net-sub-card {
  border-left: 1px solid var(--border-capsule);
  padding-left: 6px;
}

[data-theme="light"] .net-sub-card {
  background: transparent;
  border: 0;
}

.led-svg-wrap {
  width: calc(100% + 4px);
  margin: 0 -2px;
  padding-top: 2px;
}

.led-matrix-svg {
  width: 100%;
  height: 34px;
  display: block;
  overflow: visible;
}

.net-header {
  display: flex;
  align-items: baseline;
  gap: 5px;
  min-width: 0;
}

.net-direction {
  font-size: 12px;
  font-weight: 500;
  line-height: 1;
}

.net-direction.up { color: #78D9EC; }
.net-direction.down { color: #81C995; }

[data-theme="light"] .net-direction.up { color: #0284C7; }
[data-theme="light"] .net-direction.down { color: #15803D; }

.net-label {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-sub);
}

.net-speed {
  margin-left: auto;
  font-size: 13px;
  font-weight: 500;
  line-height: 1;
  color: var(--text-main);
  font-family: Consolas, monospace;
  white-space: nowrap;
}

/* SVG 几何灯珠 - 统一 6×6 正方形，描边用卡片底色形成清晰的"框" */
.svg-led {
  fill: #2A2F3A;
  opacity: 0.85;
  stroke: var(--bg-card);
  stroke-width: 1;
  transition: fill 0.35s ease, opacity 0.35s ease, filter 0.35s ease;
}

[data-theme="light"] .svg-led {
  fill: #DCE3EE;
  opacity: 0.85;
}

/* 亮起基础底色：高速整片点亮时也要有足够可见度 */
.svg-led.active-up {
  fill: #2B6E88;
  opacity: 1;
}

.svg-led.active-down {
  fill: #2F7048;
  opacity: 1;
}

[data-theme="light"] .svg-led.active-up {
  fill: #7DD3FC;
  opacity: 1;
}

[data-theme="light"] .svg-led.active-down {
  fill: #4ADE80;
  opacity: 1;
}

/* 头部高亮灯珠 + 尾随 3 颗渐变衰减灯珠 */
.svg-led.runner-head-up,
.svg-led.runner-head-down,
.svg-led.runner-tail-1-up,
.svg-led.runner-tail-1-down,
.svg-led.runner-tail-2-up,
.svg-led.runner-tail-2-down,
.svg-led.runner-tail-3-up,
.svg-led.runner-tail-3-down {
  opacity: 1;
}

/* 头部高亮灯珠 + 尾随 3 颗渐变衰减灯珠 (使用真实计算颜色，不改变 opacity，彻底避免背景底色透出闪亮) */
.svg-led.runner-head-up {
  fill: #78D9EC !important;
}

.svg-led.runner-tail-1-up {
  fill: #56A9C2 !important;
}

.svg-led.runner-tail-2-up {
  fill: #438EA8 !important;
}

.svg-led.runner-tail-3-up {
  fill: #35768F !important;
}

.svg-led.runner-head-down {
  fill: #81C995 !important;
}

.svg-led.runner-tail-1-down {
  fill: #5FAF7C !important;
}

.svg-led.runner-tail-2-down {
  fill: #4C9668 !important;
}

.svg-led.runner-tail-3-down {
  fill: #3D8258 !important;
}

/* 亮色模式：渐变从亮到淡，避免出现突兀的深色块（原先 head 用 #0284C7 太深） */
[data-theme="light"] .svg-led.runner-head-up {
  fill: #38BDF8 !important;
}
[data-theme="light"] .svg-led.runner-tail-1-up {
  fill: #5FCBF7 !important;
}
[data-theme="light"] .svg-led.runner-tail-2-up {
  fill: #7DD3FC !important;
}
[data-theme="light"] .svg-led.runner-tail-3-up {
  fill: #A7E4FD !important;
}

[data-theme="light"] .svg-led.runner-head-down {
  fill: #4ADE80 !important;
}
[data-theme="light"] .svg-led.runner-tail-1-down {
  fill: #6EE79A !important;
}
[data-theme="light"] .svg-led.runner-tail-2-down {
  fill: #86EFAC !important;
}
[data-theme="light"] .svg-led.runner-tail-3-down {
  fill: #AAF3C4 !important;
}

.device-list-card {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-sizing: border-box;
}

.search-box-row {
  padding: 0 12px 8px 12px;
  position: relative;
  display: flex;
  align-items: center;
}

.search-icon-slot {
  position: absolute;
  left: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-sub);
  pointer-events: none;
}

.device-search-input {
  width: 100%;
  height: 32px;
  line-height: normal;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  padding: 0 28px 2px 28px;
  font-size: 12px;
  color: var(--text-main);
  outline: none;
  transition: all 0.2s ease;
  box-sizing: border-box;
}

.device-search-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.15);
}

.search-clear {
  position: absolute;
  right: 20px;
  font-size: 11px;
  color: var(--text-sub);
  cursor: pointer;
  padding: 2px 4px;
}

.tree-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 4px 8px 10px 8px;
}

.group-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 8px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  color: var(--text-main);
  transition: all 0.15s;
  position: relative;
  border: 1px solid transparent;
}

.group-header:hover { background: var(--bg-capsule-hover); }

/* 拖拽指示样式 */
.group-header.drag-target-before {
  border-top: 2px solid var(--color-primary) !important;
}

.group-header.drag-target-after {
  border-bottom: 2px solid var(--color-primary) !important;
}

.group-header.drag-target-inside {
  background: var(--bg-capsule-hover) !important;
  border: 1px dashed var(--color-primary) !important;
}

.group-header.is-dragging {
  opacity: 0.4;
}

.group-chevron {
  color: var(--text-sub);
  transition: transform 0.2s ease;
}

.group-folder-icon {
  color: var(--color-primary);
  margin-right: 2px;
}

.group-name { font-size: 13px; font-weight: 500; }
.group-badge { font-size: 11px; color: var(--text-sub); }

.group-body {
  padding: 3px 0 6px 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 12px;
}

/* 1:1 对标 ssh8.py 胶囊卡片：高度 36px，内边距舒展 (虚拟渲染与硬件加速) */
.host-capsule-item {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 36px;
  padding: 0 12px;
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  border-radius: 8px;
  cursor: pointer;
  transition: background-color 0.15s ease, border-color 0.15s ease, transform 0.12s ease;
  box-sizing: border-box;
  position: relative;
  content-visibility: auto;
  contain-intrinsic-size: auto 36px;
}

.host-capsule-item:hover {
  background: var(--bg-capsule-hover);
  border-color: var(--border-card);
}

.host-capsule-item:active {
  transform: translateY(1px);
}

.host-capsule-item.drag-target-before {
  border-top: 2px solid var(--color-primary) !important;
}

.host-capsule-item.drag-target-after {
  border-bottom: 2px solid var(--color-primary) !important;
}

.host-capsule-item.is-dragging {
  opacity: 0.4;
}

.capsule-pillar {
  width: 3px;
  height: 14px;
  border-radius: 2px;
  background: var(--color-primary);
  transition: width 0.15s ease, box-shadow 0.15s ease;
}

.host-capsule-item:hover .capsule-pillar {
  width: 4px;
  box-shadow: 0 0 6px var(--color-primary);
}

.capsule-icon {
  color: var(--text-sub);
}

.capsule-title {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
}

/* 中栏：单一纯粹终端圆角大卡片 (与主题背景与边框精准契合，强制 GPU 图层圆角裁剪) */
.center-terminal-card {
  flex: 1 1 0%;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background-color: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  padding: 0 !important;
  margin: 0 !important;
  gap: 0 !important;
  overflow: hidden !important;
  transform: translateZ(0); /* 强制开启 GPU 独立渲染图层，解决硬件加速穿透导致的直角闪烁 */
  isolation: isolate;
  box-sizing: border-box;
  box-shadow: 0 1px 3px rgba(15, 23, 42, 0.05), 0 4px 14px rgba(15, 23, 42, 0.05);
}

[data-theme="light"] .center-terminal-card {
  background-color: var(--bg-card);
  border: 1px solid var(--border-card);
}

.terminal-tab-bar {
  height: 36px;
  min-height: 36px;
  background: var(--bg-card);
  border-bottom: 1px solid var(--border-card);
  border-top-left-radius: calc(var(--radius-card) - 1px);
  border-top-right-radius: calc(var(--radius-card) - 1px);
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px;
  margin: 0;
  box-sizing: border-box;
  min-width: 0;
  overflow: hidden;
}

[data-theme="light"] .terminal-tab-bar {
  background: var(--bg-card);
  border-bottom: 1px solid var(--border-card);
}

.tab-list {
  display: flex;
  align-items: center;
  gap: 6px;
  overflow-x: auto;
  position: relative;
  flex: 1 1 0%;
  min-width: 0;
}

.term-tab {
  height: 26px;
  min-height: 26px;
  max-height: 26px;
  padding: 0 8px 0 10px;
  background: var(--bg-capsule);
  border-radius: 6px;
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
  cursor: pointer;
  border: 1px solid var(--border-capsule);
  position: relative;
  z-index: 2;
  transition: background-color 0.16s ease-out, 
              color 0.16s ease-out, 
              border-color 0.16s ease-out, 
              box-shadow 0.16s ease-out;
  user-select: none;
}

[data-theme="light"] .term-tab {
  background: #DFE5EE;
  border: 1px solid #D2D8E4;
  color: #1A1C1E;
}

[data-theme="light"] .term-tab:hover {
  background: #D5DDE8;
  border-color: #CBD5E1;
}

.term-tab.active {
  background: var(--color-primary) !important;
  color: var(--color-on-primary) !important;
  border-color: var(--color-primary) !important;
  font-weight: 500;
  box-shadow: 0 1px 6px rgba(11, 87, 208, 0.3) !important;
}

[data-theme="light"] .term-tab.active {
  background: var(--color-primary) !important;
  color: #FFFFFF !important;
  border-color: var(--color-primary) !important;
}

.term-tab.locked {
  border-color: rgba(229, 193, 88, 0.4);
}

.term-tab.is-tab-dragging {
  opacity: 0.38;
  transform: scale(0.96);
  cursor: grabbing !important;
}

.term-tab.is-tab-drag-over {
  border-color: var(--color-primary) !important;
  box-shadow: 0 0 0 2px var(--color-primary), 0 2px 8px rgba(0, 0, 0, 0.2) !important;
  transform: translateY(-1.5px);
}

.tab-title {
  display: inline-flex;
  align-items: center;
  line-height: 1;
  transform: translateY(-1px);
}

.tab-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
  transform: translateY(-0.5px);
}

.tab-dot.online { background: #48BB78; box-shadow: 0 0 6px #48BB78; }
.tab-dot.offline { background: #F28B82; }

/* 1:1 对标 CustomTabCloseButton: 16px × 16px 精致微型控制钮 */
.custom-tab-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  min-width: 16px;
  min-height: 16px;
  border-radius: 4px;
  color: inherit;
  font-size: 10px;
  line-height: 1;
  padding: 0;
  margin: 0;
  cursor: pointer;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.custom-tab-btn .btn-close-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  font-size: 10px;
  transform: translateY(-0.5px);
}

.custom-tab-btn.btn-close:hover {
  background: #BA1A1A;
  color: #FFFFFF;
}

/* 标签页加锁：1:1 对标 ssh8.py CustomTabCloseButton 配色与交互 */
.custom-tab-btn.btn-lock {
  width: auto;
  height: auto;
  min-width: 0;
  min-height: 0;
  color: #C4C6D0; /* 暗色非激活: text_muted */
  opacity: 0.85;
  transition: all 0.15s ease;
}

.term-tab.active .custom-tab-btn.btn-lock {
  color: var(--color-on-primary) !important; /* 激活时: on_primary 高对比 */
  opacity: 1 !important;
}

/* 亮色模式适配 (Light Theme) */
[data-theme="light"] .custom-tab-btn.btn-lock {
  color: #55555A !important; /* 亮色非激活: text_muted */
  opacity: 0.85 !important;
}

[data-theme="light"] .term-tab.active .custom-tab-btn.btn-lock {
  color: #FFFFFF !important; /* 亮色激活（深蓝背景上）: 纯白高对比 */
  opacity: 1 !important;
}

.custom-tab-btn.btn-lock:hover {
  opacity: 1 !important;
  background: rgba(255, 255, 255, 0.12);
  border-radius: 4px;
}

[data-theme="light"] .custom-tab-btn.btn-lock:hover {
  background: rgba(0, 0, 0, 0.08);
}

[data-theme="light"] .term-tab.active .custom-tab-btn.btn-lock:hover {
  background: rgba(255, 255, 255, 0.18);
}

.tab-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

/* 状态徽标：高度26px, 宽度94px, 圆角6px */
.terminal-latency-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 26px;
  min-height: 26px;
  max-height: 26px;
  width: 94px;
  min-width: 94px;
  max-width: 94px;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 500;
  box-sizing: border-box;
  user-select: none;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all 0.2s ease;
}

.terminal-latency-badge .badge-dot {
  width: 6px;
  height: 6px;
  min-width: 6px;
  min-height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
  display: inline-block;
  animation: dotPulseGlow 1.8s ease-in-out infinite;
}

@keyframes dotPulseGlow {
  0%, 100% {
    transform: scale(1);
    opacity: 0.9;
  }
  50% {
    transform: scale(1.25);
    opacity: 1;
  }
}

/* --- 终端指示器配色规范 ---
   离线: 红色
   低于 150ms: 绿色
   高于 150ms 低于等于 250ms: 黄色
   高于 250ms: 橙色
   测速中: 浅蓝中性
*/

/* 1. 离线 (红色) */
.terminal-latency-badge.status-offline {
  background: rgba(64, 18, 22, 0.78);
  border: 1px solid rgba(220, 38, 38, 0.75);
  color: #FCA5A5 !important;
}
.terminal-latency-badge.status-offline .badge-dot {
  background: #EF4444;
  box-shadow: 0 0 6px #EF4444;
}
[data-theme="light"] .terminal-latency-badge.status-offline {
  background: #FEE2E2;
  border: 1px solid #FCA5A5;
  color: #B91C1C !important;
}
[data-theme="light"] .terminal-latency-badge.status-offline .badge-dot {
  background: #DC2626;
  box-shadow: 0 0 6px rgba(220, 38, 38, 0.45);
}

/* 2. 低于 150ms (绿色) */
.terminal-latency-badge.status-green {
  background: rgba(16, 50, 30, 0.78);
  border: 1px solid rgba(34, 197, 94, 0.75);
  color: #86EFAC !important;
}
.terminal-latency-badge.status-green .badge-dot {
  background: #22C55E;
  box-shadow: 0 0 6px #22C55E;
}
[data-theme="light"] .terminal-latency-badge.status-green {
  background: #DCFCE7;
  border: 1px solid #86EFAC;
  color: #15803D !important;
}
[data-theme="light"] .terminal-latency-badge.status-green .badge-dot {
  background: #16A34A;
  box-shadow: 0 0 6px rgba(22, 163, 74, 0.45);
}

/* 3. 高于 150ms 且低于等于 250ms (黄色) */
.terminal-latency-badge.status-yellow {
  background: rgba(60, 48, 12, 0.82);
  border: 1px solid rgba(234, 179, 8, 0.75);
  color: #FDE047 !important;
}
.terminal-latency-badge.status-yellow .badge-dot {
  background: #EAB308;
  box-shadow: 0 0 6px #EAB308;
}
[data-theme="light"] .terminal-latency-badge.status-yellow {
  background: #FEF9C3;
  border: 1px solid #FDE047;
  color: #A16207 !important;
}
[data-theme="light"] .terminal-latency-badge.status-yellow .badge-dot {
  background: #CA8A04;
  box-shadow: 0 0 6px rgba(202, 138, 4, 0.45);
}

/* 4. 高于 250ms (橙色) */
.terminal-latency-badge.status-orange {
  background: rgba(64, 32, 12, 0.82);
  border: 1px solid rgba(249, 115, 22, 0.75);
  color: #FDBA74 !important;
}
.terminal-latency-badge.status-orange .badge-dot {
  background: #F97316;
  box-shadow: 0 0 6px #F97316;
}
[data-theme="light"] .terminal-latency-badge.status-orange {
  background: #FFEDD5;
  border: 1px solid #FDBA74;
  color: #C2410C !important;
}
[data-theme="light"] .terminal-latency-badge.status-orange .badge-dot {
  background: #EA580C;
  box-shadow: 0 0 6px rgba(234, 88, 12, 0.45);
}

/* 5. 刚连接测速中 (中性青蓝) */
.terminal-latency-badge.status-measuring {
  background: rgba(20, 36, 48, 0.76);
  border: 1px solid rgba(56, 189, 248, 0.6);
  color: #BAE6FD !important;
}
.terminal-latency-badge.status-measuring .badge-dot {
  background: #38BDF8;
  box-shadow: 0 0 6px #38BDF8;
}
[data-theme="light"] .terminal-latency-badge.status-measuring {
  background: #E0F2FE;
  border: 1px solid #BAE6FD;
  color: #0369A1 !important;
}
[data-theme="light"] .terminal-latency-badge.status-measuring .badge-dot {
  background: #0284C7;
  box-shadow: 0 0 6px rgba(2, 132, 199, 0.45);
}

.btn-danger-disconnect {
  height: 28px;
  min-height: 28px;
  max-height: 28px;
  padding: 0 14px;
  font-size: 12px;
  font-weight: 500;
  background-color: #8C2A30 !important;
  color: #FFFFFF !important;
  border: 1px solid #A33840 !important;
  border-radius: var(--radius-btn);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 3px rgba(140, 42, 48, 0.2);
  white-space: nowrap;
}

.btn-danger-disconnect:hover {
  background-color: #A3343C !important;
  border-color: #B5424B !important;
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(140, 42, 48, 0.3);
}

.btn-danger-disconnect:active {
  background-color: #722026 !important;
  border-color: #722026 !important;
  transform: translateY(0);
}

[data-theme="light"] .btn-danger-disconnect {
  background-color: #B83A41 !important;
  color: #FFFFFF !important;
  border: 1px solid #A82E35 !important;
  box-shadow: 0 1px 3px rgba(184, 58, 65, 0.2);
}

[data-theme="light"] .btn-danger-disconnect:hover {
  background-color: #C8464E !important;
  border-color: #B83A41 !important;
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(184, 58, 65, 0.3);
}

/* =========================================================================
   1:1 对标 ssh8.py XTERM_TERMINAL_HTML: 终端容器底色直通与动态自定义背景
   ========================================================================= */
.terminal-viewport {
  flex: 1;
  width: 100%;
  height: 100%;
  position: relative;
  overflow: hidden !important;
  border: none !important;
  border-radius: 0 !important;
  border-bottom-left-radius: calc(var(--radius-card) - 1px) !important;
  border-bottom-right-radius: calc(var(--radius-card) - 1px) !important;
  padding: 0 !important;
  margin: 0 !important;
  box-sizing: border-box;
  background-color: transparent !important;
  transform: translateZ(0);
}

.terminal-readonly-badge {
  position: absolute;
  top: 8px;
  right: 10px;
  z-index: 30;
  height: 20px;
  padding: 0 8px;
  display: inline-flex;
  align-items: center;
  border-radius: 10px;
  border: 1px solid var(--border-capsule);
  background: var(--bg-capsule);
  color: var(--text-sub);
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.5px;
  pointer-events: none;
}

[data-theme="light"] .terminal-viewport {
  background-color: transparent !important;
  border: none !important;
  padding: 0 !important;
}

.empty-state {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  user-select: none;
  pointer-events: none;
}

.empty-icon-wrap {
  width: 144px;
  height: 144px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 4px;
  opacity: 0.92;
  transition: opacity 0.25s ease;
}

.empty-desktop-icon {
  width: 128px;
  height: 128px;
}

.empty-desktop-icon .svg-stroke {
  stroke: var(--text-title);
  opacity: 0.65;
}

.empty-desktop-icon .svg-screen {
  fill: var(--text-title);
  opacity: 0.05;
  stroke: var(--text-title);
  stroke-width: 1.5;
  stroke-dasharray: 2 2;
}

.empty-desktop-icon .svg-symbol {
  stroke: var(--primary-accent, #38BDF8);
  opacity: 0.9;
}

.empty-title { font-size: 15px; font-weight: 500; color: var(--text-title); letter-spacing: 0.3px; }
.empty-hint { font-size: 12.5px; color: var(--text-sub); }

/* 挂载容器：绝对定位 100% 充满视口。通过 visibility/z-index 切换，杜绝 display:none 导致 PTY 尺寸塌陷吞字 */
.terminal-mount-box {
  position: absolute !important;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  padding: 0 !important;
  margin: 0 !important;
  border: none !important;
  border-radius: 0 !important;
  border-bottom-left-radius: calc(var(--radius-card) - 1px) !important;
  border-bottom-right-radius: calc(var(--radius-card) - 1px) !important;
  outline: none !important;
  box-shadow: none !important;
  overflow: hidden !important;
  box-sizing: border-box;
  background-color: #0A0B0E;
  visibility: hidden;
  opacity: 0;
  pointer-events: none;
  z-index: 1;
}

.terminal-mount-box.is-active {
  visibility: visible;
  opacity: 1;
  pointer-events: auto;
  z-index: 2;
}

/* xterm.js 核心容器：直通挂载容器底色，100% 填充满容器，继承圆角遮罩 */
.terminal-mount-box .xterm {
  height: 100% !important;
  width: 100% !important;
  padding: 4px 6px !important;
  margin: 0 !important;
  border: none !important;
  border-radius: 0 !important;
  border-bottom-left-radius: calc(var(--radius-card) - 1px) !important;
  border-bottom-right-radius: calc(var(--radius-card) - 1px) !important;
  box-sizing: border-box !important;
  background-color: transparent !important;
  position: relative;
  overflow: hidden !important;
}

.terminal-mount-box .xterm-viewport {
  background-color: transparent !important;
  overflow-y: auto !important;
  border-radius: 0 !important;
  border-bottom-left-radius: calc(var(--radius-card) - 1px) !important;
  border-bottom-right-radius: calc(var(--radius-card) - 1px) !important;
}

.terminal-mount-box .xterm-screen {
  background-color: transparent !important;
  padding: 0 !important;
  box-sizing: border-box;
}

/* =========================================================================
   1:1 对标 ssh8.py 光标闪烁关键帧动画与频率节奏接管 (xterm 6)
   - 闪烁频率由 --ssh-cursor-blink-cycle 动态变量精准驱动
   - 支持 block (背景显隐与反色) / bar (阴影交替) / underline (边框交替)
   - 强制覆盖 xterm 6 内部 !important 静态规则，且严格限定在 .xterm-cursor 节点
   ========================================================================= */
@keyframes ssh-manager-cursor-block-blink {
  0%, 49.999% {
    background-color: var(--ssh-cursor-color, #81C995);
    color: var(--ssh-cursor-accent, #0A0B0E);
  }
  50%, 100% {
    background-color: transparent;
    color: var(--ssh-cursor-color, #81C995);
  }
}

@keyframes ssh-manager-cursor-bar-blink {
  0%, 49.999% { box-shadow: var(--ssh-cursor-bar-shadow, 1px 0 0 #81C995 inset); }
  50%, 100% { box-shadow: none; }
}

@keyframes ssh-manager-cursor-underline-blink {
  0%, 49.999% { border-bottom-style: solid; }
  50%, 100% { border-bottom-style: hidden; }
}

/* 覆盖 xterm 6 的默认 1s 动画与静态 !important 规则 */
.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor {
  animation-duration: var(--ssh-cursor-blink-cycle, 1060ms) !important;
  animation-timing-function: steps(1, end) !important;
  animation-iteration-count: infinite !important;
  animation-delay: 0ms !important;
}

.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-block,
.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-block.xterm-cursor-blink,
.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-block:not(.xterm-cursor-blink) {
  animation-name: ssh-manager-cursor-block-blink !important;
}

.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-bar,
.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-bar.xterm-cursor-blink {
  animation-name: ssh-manager-cursor-bar-blink !important;
}

.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-underline,
.terminal-mount-box .xterm.ssh-cursor-blink-enabled .xterm-rows .xterm-cursor.xterm-cursor-underline.xterm-cursor-blink {
  animation-name: ssh-manager-cursor-underline-blink !important;
  border-bottom-width: 1px !important;
  transform: scale(1.333333, 1.25);
  transform-origin: left bottom;
}

.terminal-mount-box .xterm.ssh-cursor-blink-disabled .xterm-rows .xterm-cursor {
  animation: none !important;
}










/* 右栏：独立圆角大卡片 (SFTP 与 备忘录) */

/* 右栏：独立圆角大卡片 (SFTP 与 备忘录) */
.right-panel-card {
  width: 280px;
  min-width: 0;
  max-width: 520px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  overflow: hidden;
  box-sizing: border-box;
  transition: width 0.28s cubic-bezier(0.16, 1, 0.3, 1), 
              flex-basis 0.28s cubic-bezier(0.16, 1, 0.3, 1), 
              opacity 0.22s cubic-bezier(0.16, 1, 0.3, 1),
              transform 0.28s cubic-bezier(0.16, 1, 0.3, 1),
              border-color 0.2s ease;
  will-change: width, flex-basis, transform, opacity;
}

.right-panel-card.is-collapsed {
  width: 0 !important;
  flex-basis: 0 !important;
  min-width: 0 !important;
  opacity: 0;
  transform: translateX(18px);
  border-color: transparent !important;
  pointer-events: none;
}

.right-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 10px;
}

.right-card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-title);
  letter-spacing: normal;
}

.sftp-container {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-card);
}

/* 顶部快捷路径导航栏 (上一级 + 路径 + 跳转) */
.sftp-nav-bar {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 8px 10px;
  box-sizing: border-box;
  width: 100%;
}

.sftp-nav-bar .btn-sftp-action {
  min-width: 48px;
  padding: 0 10px;
}

.sftp-nav-bar .btn-sftp-action:last-child {
  width: 56px;
  min-width: 56px;
  max-width: 56px;
  padding: 0;
}

.sftp-path-box {
  flex: 1 1 auto;
  min-width: 0;
}

.btn-sftp-action {
  height: 28px;
  padding: 0 8px;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius-btn);
  color: var(--color-on-primary);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  flex-shrink: 0;
  white-space: nowrap;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
}

.btn-sftp-action:hover {
  background: var(--color-primary-hover);
  transform: translateY(-1px);
}

.btn-sftp-action:active {
  background: var(--color-primary-active);
  transform: translateY(1px);
}

.sftp-path-box {
  flex: 1 1 auto;
  min-width: 0;
  height: 28px;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  padding: 0 6px;
  font-size: 12px;
  color: var(--text-main);
  outline: none;
  box-sizing: border-box;
}

/* 表头：名称与大小与权限 (支持拖拽调节宽度) */
.sftp-table-header {
  display: flex;
  align-items: center;
  padding: 6px 10px;
  font-size: 11px;
  font-weight: 500;
  color: var(--text-sub);
  border-bottom: 1px solid var(--border-card);
  user-select: none;
}

.col-name {
  position: relative;
  display: flex;
  align-items: center;
  flex-shrink: 0;
  min-width: 40px;
}

.col-size {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-shrink: 0;
  min-width: 35px;
  padding-right: 6px;
  text-align: right;
}

.col-perm {
  flex: 1;
  text-align: right;
  min-width: 64px;
}

.col-sort {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  cursor: pointer;
  user-select: none;
  transition: color 0.15s ease;
}

.col-sort:hover {
  color: var(--text-main);
}

.sort-arrow {
  font-size: 9px;
  line-height: 1;
  color: var(--color-primary);
}

.col-resizer {
  position: absolute;
  top: 0;
  right: -6px;
  width: 12px;
  bottom: 0;
  cursor: col-resize;
  z-index: 2;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent !important;
}

/* 双排点阵把手：常驻显性 2 列 × 3 行精致对齐圆点 (Double-row Grip Dots) */
.resizer-dots {
  width: 6px;
  height: 12px;
  display: block;
  background-image: radial-gradient(circle, var(--text-sub) 1px, transparent 1px);
  background-size: 3px 4px;
  background-repeat: repeat;
  background-position: center;
  opacity: 0.65;
  transition: opacity 0.15s ease, filter 0.15s ease;
  pointer-events: none;
}

.col-resizer:hover .resizer-dots,
.col-resizer:active .resizer-dots {
  opacity: 1;
  background-image: radial-gradient(circle, var(--text-main) 1px, transparent 1px);
}

.sftp-file-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 6px;
}

.sftp-loading, .sftp-empty {
  text-align: center;
  padding: 40px 10px;
  font-size: 12px;
  color: var(--text-sub);
}

.sftp-empty-title {
  display: block;
  font-size: 12px;
  font-weight: 500;
  line-height: 1.5;
  color: var(--text-main);
}

.sftp-empty-desc {
  display: block;
  margin-top: 4px;
  font-size: 11px;
  font-weight: 400;
  line-height: 1.55;
  color: var(--text-sub);
}

.file-item {
  display: flex;
  align-items: center;
  padding: 5px 8px;
  border-radius: 4px;
  cursor: pointer;
  transition: background 0.15s ease;
}

.file-item:hover {
  background: var(--bg-capsule-hover);
}

.file-item.selected {
  background: rgba(26, 86, 219, 0.15);
  border: 1px solid rgba(26, 86, 219, 0.3);
}

/* 键盘 Tab 聚焦时必须有可见焦点，鼠标点击不显示 */
.file-item:focus {
  outline: none;
}

.file-item:focus-visible {
  outline: 2px solid var(--btn-primary-bg, #A8C7FA);
  outline-offset: -2px;
}

.f-info {
  display: flex;
  align-items: center;
  gap: 6px;
  overflow: hidden;
  flex-shrink: 0;
  min-width: 0;
}

.f-name {
  font-size: 12px;
  color: var(--text-main);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.f-meta {
  display: flex;
  align-items: center;
  flex: 1;
  min-width: 0;
}

.f-size {
  font-size: 11px;
  color: var(--text-sub);
  flex-shrink: 0;
  text-align: right;
  padding-right: 6px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.f-perm {
  font-size: 10px;
  font-family: Consolas, "Cascadia Mono", monospace;
  letter-spacing: -0.2px;
  color: var(--text-sub);
  flex: 1;
  text-align: right;
  opacity: 0.7;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 底部常驻双按钮 */
.sftp-bottom-bar {
  display: flex;
  gap: 8px;
  padding: 8px 10px;
  border-top: 1px solid var(--border-card);
}

.btn-sftp-footer {
  flex: 1;
  height: 32px;
  border: none;
  border-radius: var(--radius-btn);
  background: var(--color-primary);
  color: var(--color-on-primary);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  line-height: 1;
  text-align: center;
  transition: all 0.15s ease;
}

.btn-sftp-footer .v-icon {
  display: block;
  flex: 0 0 auto;
}

.btn-sftp-footer:hover:not(:disabled) {
  background: var(--color-primary-hover);
  transform: translateY(-1px);
}

.btn-sftp-footer:active:not(:disabled) {
  background: var(--color-primary-active);
  transform: translateY(1px);
}

.btn-sftp-footer:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 备忘录 */
.memo-container {
  flex: 1;
  overflow-y: auto;
  padding: 6px 12px 14px 12px;
  display: flex;
  flex-direction: column;
  position: relative;
}

.memo-group-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 12px;
}

.memo-group-header {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 500;
  color: var(--text-main);
  padding: 4px 6px;
  border-radius: 4px;
  cursor: pointer;
  position: relative;
  border: 1px solid transparent;
  transition: all 0.15s ease;
}

.memo-group-header:hover {
  background: var(--bg-capsule-hover);
}

.memo-group-header.drag-target-before {
  border-top: 2px solid var(--color-primary) !important;
}

.memo-group-header.drag-target-after {
  border-bottom: 2px solid var(--color-primary) !important;
}

.memo-group-header.drag-target-inside {
  background: var(--bg-capsule-hover) !important;
  border: 1px dashed var(--color-primary) !important;
}

.memo-group-header.is-dragging {
  opacity: 0.4;
}

.memo-folder-icon {
  color: var(--color-primary);
  margin-right: 2px;
}

.file-item.dir .v-icon {
  color: var(--color-primary);
}

.memo-group-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-main);
}

.memo-list {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-height: 8px;
}

/* 备忘录独立圆角胶囊条 (虚拟渲染) */
.memo-capsule-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  border-radius: var(--radius-capsule);
  cursor: pointer;
  transition: background-color 0.15s ease, border-color 0.15s ease, transform 0.12s ease;
  position: relative;
  box-sizing: border-box;
  content-visibility: auto;
  contain-intrinsic-size: auto 32px;
}

.memo-capsule-item:hover {
  background: var(--bg-capsule-hover);
  border-color: var(--border-card);
}

.memo-capsule-item:active {
  transform: translateY(1px);
}

.memo-capsule-item.drag-target-before {
  border-top: 2px solid var(--color-primary) !important;
}

.memo-capsule-item.drag-target-after {
  border-bottom: 2px solid var(--color-primary) !important;
}

.memo-capsule-item.is-dragging {
  opacity: 0.4;
}

.memo-pillar {
  width: 3px;
  height: 12px;
  border-radius: 2px;
  background: var(--color-primary);
  transition: width 0.15s ease, box-shadow 0.15s ease;
}

.memo-capsule-item:hover .memo-pillar {
  width: 4px;
  box-shadow: 0 0 6px var(--color-primary);
}

.memo-icon {
  color: var(--text-sub);
}

.memo-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
  flex: 1;
}

.btn-switch-white-pill {
  font-family: var(--app-font-family);
  height: 28px;
  width: 56px;
  min-width: 56px;
  max-width: 56px;
  box-sizing: border-box;
  padding: 0;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-on-primary);
  background: var(--color-primary);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-btn);
  cursor: pointer;
  display: inline-flex;
  flex-direction: row;
  align-items: center;
  justify-content: center;
  text-align: center;
  white-space: nowrap;
  word-break: keep-all;
  overflow: visible;
  line-height: 26px;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1);
}

.btn-switch-white-pill:hover {
  background: var(--color-primary-hover);
  transform: translateY(-1px);
}

.btn-switch-white-pill:active {
  background: var(--color-primary-active);
  transform: translateY(1px);
}

.memo-bottom-action {
  margin-top: auto;
  padding-top: 8px;
}

.btn-bottom-add-pill {
  font-family: var(--app-font-family);
  width: 100%;
  height: 32px;
  background: var(--color-primary);
  color: var(--color-on-primary);
  font-size: 12px;
  font-weight: 500;
  border: none;
  border-radius: var(--radius-capsule);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}

.btn-bottom-add-pill:hover {
  background: var(--color-primary-hover);
  transform: translateY(-1px);
}

.btn-bottom-add-pill:active {
  background: var(--color-primary-active);
  transform: translateY(1px);
}

.memo-actions {
  display: flex;
  gap: 4px;
  margin-left: 6px;
}

.btn-memo-act {
  width: 20px;
  height: 20px;
  background: transparent;
  border: none;
  color: #8E9099;
  cursor: pointer;
  font-size: 11px;
  border-radius: 3px;
}

.btn-memo-act:hover { background: #2A303F; color: #FFF; }
.btn-memo-act.danger:hover { background: #B3261E; color: #FFF; }

/* 悬浮传输监视器 (SFTPTransferMonitorPopover) */
.transfer-monitor-popover {
  position: absolute;
  top: 48px;
  right: 15px;
  width: 368px;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.3);
  z-index: 9998;
  display: flex;
  flex-direction: column;
  overflow: visible !important;
  transition: background-color 0.25s ease, border-color 0.25s ease;
}

/* 1:1 对标 ssh8.py ARROW_HEIGHT = 9, ARROW_WIDTH = 14: 气泡顶端指示性小三角 */
.transfer-monitor-popover::before {
  content: "";
  position: absolute;
  top: -8px;
  right: var(--arrow-right, 85px);
  width: 0;
  height: 0;
  border-left: 8px solid transparent;
  border-right: 8px solid transparent;
  border-bottom: 8px solid var(--border-card);
  z-index: 10000;
  pointer-events: none;
}

.transfer-monitor-popover::after {
  content: "";
  position: absolute;
  top: -7px;
  right: calc(var(--arrow-right, 85px) + 1px);
  width: 0;
  height: 0;
  border-left: 7px solid transparent;
  border-right: 7px solid transparent;
  border-bottom: 7px solid var(--bg-sub-card);
  z-index: 10001;
  pointer-events: none;
}

.transfer-popover-header {
  padding: 8px 12px;
  background: var(--bg-sub-card);
  border-bottom: 1px solid var(--border-card);
  border-top-left-radius: calc(var(--radius-card) - 1px);
  border-top-right-radius: calc(var(--radius-card) - 1px);
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-title);
}

.popover-header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.btn-popover-clear {
  height: 22px;
  padding: 0 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-on-primary);
  background-color: var(--color-primary);
  border: 1px solid var(--color-primary);
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.btn-popover-clear:hover:not(:disabled) {
  background-color: var(--color-primary-hover);
}

.btn-popover-clear:disabled {
  opacity: 0.45;
  cursor: not-allowed;
  filter: grayscale(40%);
}

.popover-close { cursor: pointer; color: var(--text-sub); }
.popover-close:hover { color: var(--text-main); }

.transfer-task-list {
  max-height: 260px;
  overflow-y: auto;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  background: var(--bg-card);
  border-bottom-left-radius: calc(var(--radius-card) - 1px);
  border-bottom-right-radius: calc(var(--radius-card) - 1px);
}

.no-task {
  text-align: center;
  padding: 16px;
  font-size: 11px;
  color: var(--text-sub);
}

.transfer-task-item {
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  border-radius: var(--radius-btn);
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.task-info {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
}

.task-badge {
  padding: 1px 4px;
  border-radius: 3px;
  font-size: 9px;
  font-weight: 600;
}

.task-badge.up { background: rgba(34, 211, 238, 0.16); color: #67E8F9; }
.task-badge.down { background: rgba(45, 212, 191, 0.16); color: #5EEAD4; }

[data-theme="light"] .task-badge.up { background: rgba(8, 145, 178, 0.1); color: #0E7490; }
[data-theme="light"] .task-badge.down { background: rgba(13, 148, 136, 0.1); color: #0F766E; }

.task-name {
  flex: 1;
  margin: 0 6px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-main);
  font-size: 12px;
}

.task-speed { font-size: 11px; color: var(--text-main); font-family: monospace; }

.task-progress-bar {
  height: 5px;
  background: var(--bg-track);
  border-radius: 2px;
  overflow: visible;
}

.task-progress-fill {
  height: 8px;
  margin-top: -1.5px;
  background: var(--color-primary);
  border-radius: 2px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.26);
  transition: width 0.2s;
}

.task-status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  color: var(--text-sub);
  line-height: 1;
}

.task-status-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  line-height: 1;
}

.status-tag {
  display: inline-flex;
  align-items: center;
  line-height: 1;
}

.task-error {
  margin-top: 2px;
  color: #F28B82;
  font-size: 9px;
  line-height: 1.25;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.status-tag.completed { color: #81C995; font-weight: 500; }
.status-tag.failed { color: #F28B82; font-weight: 500; }
.status-tag.cancelled { color: #A8A8B3; font-weight: 500; }
.status-tag.running { color: #78D9EC; }

[data-theme="light"] .status-tag.completed { color: #15803D; }
[data-theme="light"] .status-tag.failed { color: #DC2626; }
[data-theme="light"] .status-tag.cancelled { color: var(--text-sub); }
[data-theme="light"] .status-tag.running { color: #0284C7; }

.task-status-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.btn-task-cancel {
  height: 24px;
  min-width: 52px;
  padding: 0 10px;
  border: 1px solid var(--danger-border);
  border-radius: var(--radius-btn);
  background: var(--danger-bg) !important;
  color: var(--danger-text) !important;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
}

.btn-task-cancel:hover {
  background: var(--danger-hover) !important;
  border-color: var(--danger-hover) !important;
  color: var(--danger-text) !important;
  transform: translateY(-1px);
}

.btn-task-cancel:active {
  background: var(--danger-active) !important;
  border-color: var(--danger-active) !important;
  transform: translateY(1px);
}

/* 视图 2：主机管理大盘 (平滑推进淡入) */
.mgmt-view {
  flex: 1;
  overflow-y: auto;
  padding: 24px;
  background: transparent;
  position: relative;
  z-index: 1;
  animation: view-flow-enter 0.22s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes view-flow-enter {
  from {
    opacity: 0;
    transform: translateY(8px) translateZ(0);
  }
  to {
    opacity: 1;
    transform: translateY(0) translateZ(0);
  }
}

.mgmt-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
}

.mgmt-title { font-size: 18px; font-weight: 600; color: var(--text-main); }
.mgmt-sub { font-size: 12px; color: var(--text-sub); }

.btn-add-host {
  height: 30px;
  padding: 0 16px;
  background: var(--color-primary);
  color: var(--color-on-primary);
  border: none;
  border-radius: var(--radius-btn);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}

.btn-add-host:hover {
  background: var(--color-primary-hover);
  transform: translateY(-1px);
}

.btn-sec-act {
  height: 30px;
  padding: 0 14px;
  background: var(--color-primary);
  color: var(--color-on-primary);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-btn);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}

.btn-sec-act:hover:not(:disabled) {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
  transform: translateY(-1px);
}

.btn-sec-act:active:not(:disabled) {
  background: var(--color-primary-active);
  transform: translateY(0);
}

.btn-sec-act:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  transform: none;
  box-shadow: none;
}

.mgmt-cols-picker {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: var(--bg-capsule);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-btn);
  padding: 3px 6px;
}

.cols-label {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-sub);
  margin-right: 4px;
  margin-left: 4px;
}

.btn-col-choice {
  height: 26px;
  min-width: 42px;
  padding: 0 10px;
  border-radius: 5px;
  border: none;
  background: transparent;
  color: var(--text-sub);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
}

.btn-col-choice:hover {
  color: var(--text-main);
  background: var(--bg-capsule-hover);
}

.btn-col-choice.active {
  background: var(--color-primary);
  color: var(--color-on-primary);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2);
}

.cards-grid {
  display: grid;
  gap: 16px;
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.cards-grid.cols-1 {
  grid-template-columns: 1fr;
}

.cards-grid.cols-2 {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.cards-grid.cols-3 {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.cards-grid.cols-4 {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

/* 响应式兜底适配 */
@media (max-width: 1400px) {
  .cards-grid.cols-4 {
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  }
}

.host-card {
  position: relative;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  overflow: hidden;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.4), 0 8px 24px rgba(0, 0, 0, 0.35);
  transition: transform 0.2s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.2s, box-shadow 0.2s;
  content-visibility: auto;
  contain-intrinsic-size: auto 240px;
}

[data-theme="light"] .host-card {
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06), 0 3px 12px rgba(0, 0, 0, 0.05);
}

.host-card:hover {
  transform: translateY(-2px);
  border-color: #79747E;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.45), 0 12px 32px rgba(0, 0, 0, 0.4);
}

[data-theme="light"] .host-card:hover {
  border-color: #85858E;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.09);
}

/* 拖拽排序交互视觉反馈 */
.host-card.is-dragging {
  opacity: 0.45;
  transform: scale(0.98);
  border-style: dashed;
  border-color: var(--color-primary);
  cursor: grabbing;
}

.host-card.is-drag-over {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary), 0 8px 20px rgba(0, 0, 0, 0.25);
  transform: translateY(-4px);
}

/* 通知点击高亮：边框平缓呼吸灯，循环3次 (每轮 1.6s，总计 4.8s) */
.host-card.breathing-highlight {
  animation: cardBreathingGlow 1.6s cubic-bezier(0.4, 0, 0.2, 1) 3;
  z-index: 10;
  position: relative;
}

@keyframes cardBreathingGlow {
  0% {
    border-color: var(--border-card);
    box-shadow: 0 0 0 0 rgba(32, 87, 166, 0), 0 2px 6px rgba(0, 0, 0, 0.08);
    transform: translateY(0);
  }
  50% {
    border-color: var(--color-primary);
    box-shadow: 
      0 0 0 3px color-mix(in srgb, var(--color-primary) 55%, transparent),
      0 0 22px 6px color-mix(in srgb, var(--color-primary) 40%, transparent),
      0 8px 24px rgba(0, 0, 0, 0.16);
    transform: translateY(-2px);
  }
  100% {
    border-color: var(--border-card);
    box-shadow: 0 0 0 0 rgba(32, 87, 166, 0), 0 2px 6px rgba(0, 0, 0, 0.08);
    transform: translateY(0);
  }
}

.card-pillar {
  position: absolute;
  top: 0; left: 0; bottom: 0;
  width: 4px;
  background: var(--color-primary);
}

.card-body {
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.card-row1 {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.card-name { font-size: 14px; font-weight: 600; color: var(--text-title); }
.card-latency {
  font-size: 11px;
  font-family: monospace;
  color: #48BB78;
  background: rgba(72, 187, 120, 0.12);
  padding: 2px 6px;
  border-radius: 4px;
}

.card-addr { font-size: 12px; color: var(--text-sub); }

.card-meta {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 4px;
  font-size: 11px;
  color: var(--text-sub);
}

.traffic-section {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-top: 4px;
}

.traffic-label {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11px;
  color: var(--text-main);
  line-height: 1.2;
}

.traffic-text-main {
  font-size: 11px;
  font-weight: 500;
  color: var(--text-main);
}

.traffic-audit-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: 22px;
  padding: 0 8px;
  border-radius: var(--radius-btn, 6px);
  border: 1px solid rgba(56, 189, 248, 0.3);
  background: rgba(56, 189, 248, 0.08);
  color: #38BDF8;
  font-size: 11px;
  font-weight: 500;
  font-family: inherit;
  line-height: 1;
  letter-spacing: 0.2px;
  cursor: pointer;
  box-sizing: border-box;
  white-space: nowrap;
  outline: none;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
  -webkit-font-smoothing: antialiased;
}

.traffic-audit-btn .audit-icon {
  font-size: 11px;
  line-height: 1;
  opacity: 0.9;
  transition: transform 0.25s ease;
}

.traffic-audit-btn:hover {
  background: rgba(56, 189, 248, 0.18);
  border-color: rgba(56, 189, 248, 0.55);
  color: #7DD3FC;
  transform: translateY(-1px);
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.25);
}

.traffic-audit-btn:hover .audit-icon {
  transform: rotate(45deg);
}

.traffic-audit-btn:active {
  transform: translateY(0);
  background: rgba(56, 189, 248, 0.25);
}

/* 日间模式：精细微立体胶囊按钮，深蓝字样与微描边，杜绝发虚并与卡片整体质感呼应 */
[data-theme="light"] .traffic-audit-btn {
  background: #EFF6FF;
  border-color: #BFDBFE;
  color: #1D4ED8;
  box-shadow: 0 1px 2px rgba(29, 78, 216, 0.06);
}

[data-theme="light"] .traffic-audit-btn:hover {
  background: #DBEAFE;
  border-color: #93C5FD;
  color: #1E40AF;
  box-shadow: 0 2px 4px rgba(29, 78, 216, 0.12);
}

[data-theme="light"] .traffic-audit-btn:active {
  background: #BFDBFE;
}

.card-latency.is-tcp {
  color: #60A5FA;
  background: rgba(96, 165, 250, 0.15);
  border: 1px solid rgba(96, 165, 250, 0.3);
}

.card-latency.is-icmp {
  color: #38BDF8;
  background: rgba(56, 189, 248, 0.15);
  border: 1px solid rgba(56, 189, 248, 0.3);
}

.card-latency.is-err {
  color: #F87171;
  background: rgba(248, 113, 113, 0.15);
  border: 1px solid rgba(248, 113, 113, 0.3);
}

/* 统一设计规范的进度条系统 (高度 16px、圆角 8px、文字居中清晰) */
.unified-traffic-bar {
  position: relative;
  height: 16px;
  border-radius: 2px;
  background: var(--bg-track);
  border: 1px solid var(--border-card);
  overflow: visible;
}

.unified-traffic-fill {
  position: absolute;
  left: 0;
  top: 50%;
  height: 20px;
  border-radius: 2px;
  background: var(--metric-cpu-from);
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.26);
  transform: translateY(-50%);
  transition: width 0.35s ease;
  z-index: 1;
}

.unified-traffic-fill.warn {
  background: #D97706;
}

.unified-traffic-fill.danger {
  background: #DC2626;
}

[data-theme="light"] .unified-traffic-fill {
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.14);
}

.unified-traffic-pct {
  position: absolute;
  left: 0;
  right: 0;
  top: 0;
  bottom: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 600;
  font-family: Consolas, monospace;
  color: #FFFFFF;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.85);
  line-height: 1;
  pointer-events: none;
  z-index: 2;
}

/* 无限流量流光条 (平滑无限循环、自适应日间/夜间配色) */
.unified-traffic-bar.infinite {
  background: #181C26;
  justify-content: center;
  border-color: rgba(56, 189, 248, 0.25);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.2);
  overflow: hidden;
}

.infinite-flow {
  position: absolute;
  top: 0; left: 0; right: 0; bottom: 0;
  width: 100%;
  height: 100%;
  background-image: repeating-linear-gradient(
    45deg,
    rgba(56, 189, 248, 0.22) 0,
    rgba(56, 189, 248, 0.22) 10px,
    transparent 10px,
    transparent 20px
  );
  background-size: 28.284px 28.284px;
  animation: seamlessStripe 1s linear infinite;
}

@keyframes seamlessStripe {
  0% {
    background-position: 0 0;
  }
  100% {
    background-position: 28.284px 0;
  }
}

.infinity-badge {
  position: relative;
  z-index: 2;
  font-size: 10px;
  font-weight: 600;
  color: #38BDF8;
  letter-spacing: 0.5px;
  line-height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.85);
}

/* 日间模式：内嵌式微凹灰底槽，搭配雅致中性斜纹流动，色彩沉稳且不与白卡片突兀冲撞 */
[data-theme="light"] .unified-traffic-bar.infinite {
  background: #E8E8ED;
  border-color: #D2D2D8;
  box-shadow: inset 0 1px 2.5px rgba(0, 0, 0, 0.08);
}

[data-theme="light"] .infinite-flow {
  background-image: repeating-linear-gradient(
    45deg,
    rgba(140, 145, 160, 0.18) 0,
    rgba(140, 145, 160, 0.18) 10px,
    transparent 10px,
    transparent 20px
  );
}

[data-theme="light"] .infinity-badge {
  color: #3C4252;
  text-shadow: none;
  font-weight: 600;
}

.card-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
}

.card-actions-left {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  flex: 1;
  min-width: 0;
}

.card-actions-right {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-shrink: 0;
  margin-left: auto;
}

/* 统一卡片操作按钮：以 SFTP 传输按钮为模板的统一视觉设计，统一宽度与厚度 */
.btn-card-act {
  height: 28px;
  min-width: 68px;
  padding: 0 10px;
  font-size: 11px;
  font-weight: 500;
  border-radius: var(--radius-btn);
  border: 1px solid var(--color-primary);
  background: var(--color-primary);
  color: var(--color-on-primary);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  box-sizing: border-box;
  white-space: nowrap;
  gap: 4px;
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
}

.btn-card-act:hover {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
}

.btn-card-act:active {
  background: var(--color-primary-active);
  transform: translateY(1px);
}

/* 打开 SSH: 突出主操作 */
.btn-card-act.btn-ssh {
  font-weight: 600;
}

/* 危险删除按钮：克制雅致的非鲜红配色，同时适配深浅主题 */
.btn-card-act.btn-del,
.btn-del,
.btn-danger {
  height: 28px;
  padding: 0 16px;
  border-radius: var(--radius-btn);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  background: #933339 !important;
  border: 1px solid #A33D44 !important;
  color: #FFFFFF !important;
  box-shadow: 0 1px 3px rgba(147, 51, 57, 0.2);
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}

.btn-card-act.btn-del:hover,
.btn-del:hover,
.btn-danger:hover {
  background: #A83C44 !important;
  border-color: #B8474F !important;
  transform: translateY(-1px);
}

.btn-card-act.btn-del:active,
.btn-del:active,
.btn-danger:active {
  background: #7D292E !important;
  border-color: #7D292E !important;
  transform: translateY(0);
}

[data-theme="light"] .btn-card-act.btn-del,
[data-theme="light"] .btn-del,
[data-theme="light"] .btn-danger {
  background: #C24148 !important;
  border-color: #B3343B !important;
  color: #FFFFFF !important;
}

[data-theme="light"] .btn-card-act.btn-del:hover,
[data-theme="light"] .btn-del:hover,
[data-theme="light"] .btn-danger:hover {
  background: #D14D54 !important;
  border-color: #C24148 !important;
}

/* 编辑主机弹窗专有排版样式 */
.modal-host-edit {
  width: 560px !important;
  height: 620px !important;
  max-height: 88vh !important;
}

.modal-scrollable {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 16px 16px 40px 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.form-section-card {
  background: rgba(255, 255, 255, 0.025);
  border: 1px solid var(--border-card);
  border-radius: 8px;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

[data-theme="light"] .form-section-card {
  background: #F8F9FC;
  border-color: #E2E4E9;
}

.modal-section-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-primary);
  display: flex;
  align-items: center;
  gap: 6px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  padding-bottom: 6px;
  margin-bottom: 2px;
  letter-spacing: 0.5px;
}

.section-title-icon {
  font-size: 8px;
  color: var(--color-primary);
}

.field-col {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.grid-host-port {
  display: grid;
  grid-template-columns: 1fr 75px;
  gap: 12px;
  align-items: start;
}

.grid-2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  align-items: start;
}

.grid-3 {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 12px;
  align-items: start;
}

.text-center {
  text-align: center;
}

.label-with-addon {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.input-combined-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}

.w-28 {
  width: 110px !important;
  flex-shrink: 0;
}

.w-24 {
  width: 85px !important;
  flex-shrink: 0;
}

.w-20 {
  width: 72px !important;
  flex-shrink: 0;
}

.infinite-addon-check {
  margin-left: 2px;
  flex-shrink: 0;
  white-space: nowrap;
  font-size: 12px;
  cursor: pointer;
  user-select: none;
}

.reset-time-row {
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  padding: 4px 10px;
}

.reset-day-input {
  width: 60px !important;
  height: 28px !important;
  padding: 0 4px !important;
}

.reset-digit-input {
  width: 50px !important;
  height: 28px !important;
  padding: 0 4px !important;
}

.time-prefix, .time-suffix, .time-colon {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
}

.time-hint {
  font-size: 11px;
  color: var(--text-sub);
  margin-left: 4px;
}

.btn-sec-act.btn-browse {
  height: 32px;
  padding: 0 14px;
  font-size: 12px;
  font-weight: 500;
  background: var(--bg-capsule);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  color: var(--text-main);
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.btn-sec-act.btn-browse:hover {
  background: var(--bg-capsule-hover);
  color: #FFFFFF;
  border-color: var(--color-primary);
}

/* 全局拟态右键菜单 (无缝融入卡片与工作区统一 MD3 质感体系) */
.context-menu {
  position: fixed;
  z-index: 9999;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-btn);
  padding: 5px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45), 0 8px 32px rgba(0, 0, 0, 0.4);
  min-width: 170px;
}

.menu-item {
  padding: 6px 12px;
  font-size: 12px;
  color: var(--text-main);
  border-radius: 4px;
  cursor: pointer;
  transition: background-color 0.15s ease, color 0.15s ease;
}

.menu-item:hover { 
  background: var(--bg-capsule-hover); 
  color: var(--color-primary); 
}

.menu-item.danger:hover { 
  background: var(--danger-bg); 
  color: var(--danger-text); 
}

.menu-item.disabled {
  color: var(--text-sub) !important;
  opacity: 0.5;
  cursor: not-allowed !important;
}

.menu-item.disabled:hover {
  background: transparent !important;
  color: var(--text-sub) !important;
}

.menu-divider { 
  height: 1px; 
  background: var(--border-capsule); 
  margin: 4px 2px; 
}

[data-theme="light"] .context-menu {
  background: var(--bg-card);
  border-color: var(--border-card);
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.1), 0 8px 24px rgba(0, 0, 0, 0.08);
}

/* 模态弹窗系统 (完全绑定 Dark / Light 模板) */
.modal-overlay {
  position: fixed;
  top: 0; left: 0; right: 0; bottom: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10000;
}

/* 文件浏览器会从设置、主机编辑、备份等弹窗中打开，必须是最高层。 */
.modal-overlay.local-picker-overlay {
  z-index: 300000;
  /* 避免 blur 背景层在按钮点击时反复重绘产生闪烁。 */
  backdrop-filter: none !important;
  -webkit-backdrop-filter: none !important;
  background: rgba(0, 0, 0, 0.58);
}

.local-picker-overlay button:focus {
  outline: none !important;
}

/* 鼠标点击不显示外框，但键盘 Tab 聚焦时必须可见，否则文件浏览器无法键盘操作。 */
.local-picker-overlay button:focus-visible {
  outline: 2px solid var(--btn-primary-bg, #A8C7FA) !important;
  outline-offset: 2px;
  box-shadow: none !important;
}

.local-picker-overlay button {
  -webkit-tap-highlight-color: transparent;
}

.transfer-conflict-overlay {
  z-index: 310000;
}

/* 模态弹窗 Vue Transition 优雅过渡 (纯垂直位移 + 纯透明度，杜绝 scale 导致的亚像素文字抖动) */
.modal-fade-enter-from {
  opacity: 0 !important;
  backdrop-filter: blur(0px) !important;
}
.modal-fade-leave-to {
  opacity: 0 !important;
  backdrop-filter: blur(0px) !important;
}

.modal-fade-enter-active {
  transition: opacity 0.22s cubic-bezier(0.16, 1, 0.3, 1), backdrop-filter 0.22s cubic-bezier(0.16, 1, 0.3, 1);
}
.modal-fade-leave-active {
  transition: opacity 0.18s cubic-bezier(0.4, 0, 0.2, 1), backdrop-filter 0.18s cubic-bezier(0.4, 0, 0.2, 1);
}

.modal-fade-enter-from .modal-card {
  opacity: 0;
  transform: translateY(12px) translateZ(0);
}
.modal-fade-leave-to .modal-card {
  opacity: 0;
  transform: translateY(8px) translateZ(0);
}

.modal-fade-enter-active .modal-card {
  transition: transform 0.22s cubic-bezier(0.16, 1, 0.3, 1), opacity 0.2s ease-out;
}
.modal-fade-leave-active .modal-card {
  transition: transform 0.18s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.18s cubic-bezier(0.4, 0, 0.2, 1);
}

/* 顶栏气泡卡片优雅呼出 (popover-fade) */
.popover-fade-enter-active {
  transition: transform 0.18s cubic-bezier(0.16, 1, 0.3, 1), opacity 0.18s ease-out;
}
.popover-fade-leave-active {
  transition: transform 0.14s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.14s ease-in;
}
.popover-fade-enter-from {
  opacity: 0;
  transform: translateY(-6px) translateZ(0);
}
.popover-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px) translateZ(0);
}

/* 全局右键菜单微动效 (context-fade) */
.context-fade-enter-active {
  transition: opacity 0.12s ease-out, transform 0.12s cubic-bezier(0.16, 1, 0.3, 1);
}
.context-fade-leave-active {
  transition: opacity 0.08s ease-in;
}
.context-fade-enter-from {
  opacity: 0;
  transform: scale(0.98);
}
.context-fade-leave-to {
  opacity: 0;
}

/* 右侧栏平滑抽屉过渡 (drawer-slide) - 优化收起缓动，告别半路掐断感 */
.drawer-slide-enter-active {
  transition: transform 0.26s cubic-bezier(0.16, 1, 0.3, 1), opacity 0.24s cubic-bezier(0.16, 1, 0.3, 1);
}
.drawer-slide-leave-active {
  transition: transform 0.22s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.22s cubic-bezier(0.4, 0, 0.2, 1);
}

.drawer-slide-enter-from {
  transform: translateX(40px) translateZ(0);
  opacity: 0;
}
.drawer-slide-leave-to {
  transform: translateX(40px) translateZ(0);
  opacity: 0;
}

.drawer-fade-enter-active {
  transition: opacity 0.22s ease-out;
}
.drawer-fade-leave-active {
  transition: opacity 0.18s ease-in;
}

.drawer-fade-enter-from,
.drawer-fade-leave-to {
  opacity: 0;
}

/* 分组折叠手风琴平滑过渡 (accordion) */
.accordion-enter-active {
  transition: opacity 0.2s cubic-bezier(0.16, 1, 0.3, 1), transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  transform-origin: top center;
}
.accordion-leave-active {
  transition: opacity 0.16s cubic-bezier(0.4, 0, 0.2, 1), transform 0.16s cubic-bezier(0.4, 0, 0.2, 1);
  transform-origin: top center;
}

.accordion-enter-from {
  opacity: 0;
  transform: translateY(-8px);
}
.accordion-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}

.modal-card {
  width: 480px;
  background: var(--bg-modal);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  transform: translateY(0) translateZ(0);
  backface-visibility: hidden;
  -webkit-backface-visibility: hidden;
  transition: background-color 0.25s ease, border-color 0.25s ease;
}

.modal-card.modal-small {
  width: 380px;
}

.ssh-password-modal {
  width: 360px;
}

.ssh-password-host {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding: 9px 11px;
  margin-bottom: 12px;
  border: 1px solid var(--border-card);
  border-radius: var(--radius-btn);
  background: var(--bg-capsule);
}

.ssh-password-host-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-main);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ssh-password-host-addr {
  font-size: 11px;
  color: var(--text-sub);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ssh-password-error {
  margin-top: 5px;
  font-size: 11px;
  color: #ef4444;
}

.ssh-remember-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--text-main);
  cursor: pointer;
  user-select: none;
}

.ssh-remember-row input[type="checkbox"] {
  width: 14px;
  height: 14px;
  margin: 0;
  accent-color: var(--color-primary);
  cursor: pointer;
}

.ssh-password-hint {
  margin: 7px 0 0;
  font-size: 10px;
  line-height: 1.5;
  color: var(--text-sub);
}

.confirm-modal-body {
  padding: 20px 18px !important;
}

.confirm-modal-text {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-main);
  word-break: break-word;
  white-space: pre-wrap;
}

.modal-header {
  height: 44px;
  padding: 0 16px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  background: var(--bg-card);
  border-bottom: 1px solid var(--border-card);
}

.modal-title { font-size: 13px; font-weight: 600; color: var(--text-title); }
.modal-close { font-size: 14px; color: var(--text-sub); cursor: pointer; padding: 4px; transition: color 0.15s; }
.modal-close:hover { color: var(--text-main); }

.modal-body {
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  background: var(--bg-modal);
}

.form-row { display: flex; flex-direction: column; gap: 5px; }
.form-row.grid-host-port.is-serial,
.grid-host-port.is-serial {
  grid-template-columns: 1fr 195px !important;
}
.combo-baud-wrapper {
  position: relative;
  display: flex;
  align-items: center;
  width: 100%;
}

.combo-baud-input {
  width: 100% !important;
  padding-right: 28px !important;
  text-align: left !important;
  padding-left: 10px !important;
  background-image: none !important;
  -webkit-appearance: none !important;
  -moz-appearance: textfield !important;
}

.combo-baud-arrow {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  width: 16px;
  height: 16px;
  background: transparent !important;
  border: none !important;
  outline: none !important;
  padding: 0 !important;
  color: var(--color-primary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: transform 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}

.combo-baud-arrow svg {
  transition: transform 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}

.combo-baud-arrow.rotated svg {
  transform: rotate(180deg);
}

.modal-host-edit .select-dropdown,
.modal-host-edit .combo-baud-dropdown {
  max-height: 175px !important;
}

.combo-baud-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  width: 100%;
  min-width: 100%;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: 6px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  z-index: 99999;
  padding: 4px;
  max-height: 220px;
  overflow-y: auto;
  box-sizing: border-box;
}

[data-theme="light"] .combo-baud-dropdown {
  background: var(--bg-card);
  border-color: var(--border-card);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
}

.combo-baud-option {
  padding: 6px 10px;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  color: var(--text-main);
  cursor: pointer;
  font-size: 12px;
  transition: background 0.15s ease, color 0.15s ease;
}

.combo-baud-option:hover {
  background: var(--bg-capsule-hover);
  color: var(--color-primary);
}

[data-theme="light"] .combo-baud-option:hover {
  background: #EFF6FF;
  color: #1A56DB;
}

.combo-baud-option.active {
  background: var(--color-primary);
  color: var(--color-on-primary);
  font-weight: 600;
}

.form-row.grid-host-port.is-serial,
.grid-host-port.is-serial {
  grid-template-columns: 1fr 170px !important;
}
.form-row.grid-host-port {
  display: grid !important;
  grid-template-columns: 1fr 75px !important;
  gap: 12px;
  align-items: start;
}
.form-row.grid-2 {
  display: grid !important;
  grid-template-columns: 1fr 1fr !important;
  gap: 12px;
  align-items: start;
}
.form-row.grid-3 {
  display: grid !important;
  grid-template-columns: 1fr 1fr 1fr !important;
  gap: 12px;
  align-items: start;
}
.form-row label { font-size: 11px; color: var(--text-sub); }

.field-hint {
  font-size: 10px;
  line-height: 1.55;
  color: var(--text-sub);
  opacity: 0.9;
}

.field-hint code {
  font-family: Consolas, monospace;
  color: var(--color-primary);
  background: rgba(120, 217, 236, 0.08);
  padding: 0 3px;
  border-radius: 3px;
}

/* 密码显示按钮统一使用自绘图标，隐藏 WebView2/Edge 原生黑色眼睛 */
input[type="password"]::-ms-reveal,
input[type="password"]::-ms-clear {
  display: none !important;
}

.password-field-wrap {
  position: relative;
  width: 100%;
}

.password-field-wrap .modal-input {
  padding-right: 36px;
}

.password-toggle-btn {
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-sub);
  cursor: pointer;
  border-radius: 4px;
  transition: color 0.15s ease, background 0.15s ease;
}

.password-toggle-btn:hover {
  color: var(--color-primary);
  background: var(--bg-capsule-hover);
}

.password-toggle-btn svg {
  width: 15px;
  height: 15px;
}

.traffic-manual-check {
  margin-top: 2px;
  font-size: 11px;
  line-height: 1.5;
}

.modal-input {
  height: 32px;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  padding: 0 10px;
  font-size: 12px;
  color: var(--text-main);
  outline: none;
  width: 100%;
  box-sizing: border-box;
  transition: border-color 0.2s ease, box-shadow 0.2s ease, background-color 0.2s ease;
}

select,
select.modal-input,
select.sftp-quick-select {
  appearance: none !important;
  -webkit-appearance: none !important;
  -moz-appearance: none !important;
  background-color: var(--bg-input) !important;
  border: 1px solid var(--border-input) !important;
  border-radius: var(--radius-btn) !important;
  color: var(--text-main) !important;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%2378D9EC' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E") !important;
  background-repeat: no-repeat !important;
  background-position: right 10px center !important;
  padding-right: 28px !important;
  cursor: pointer;
  outline: none !important;
  transition: border-color 0.2s ease, box-shadow 0.2s ease, background-color 0.2s ease;
}

[data-theme="light"] select,
[data-theme="light"] select.modal-input,
[data-theme="light"] select.sftp-quick-select {
  background-color: var(--bg-input) !important;
  border-color: var(--border-input) !important;
  color: var(--text-main) !important;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%231A56DB' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E") !important;
}

select:focus,
select.modal-input:focus,
select.sftp-quick-select:focus {
  border-color: var(--color-primary) !important;
  box-shadow: 0 0 0 2px rgba(120, 217, 236, 0.15) !important;
}

select option {
  background-color: #181B22 !important;
  color: #E2E2E9 !important;
  padding: 8px 12px !important;
}

[data-theme="light"] select option {
  background-color: #FFFFFF !important;
  color: #0F172A !important;
}

input[type="checkbox"] {
  appearance: none;
  -webkit-appearance: none;
  width: 16px;
  height: 16px;
  border-radius: 4px;
  border: 1.5px solid var(--border-input);
  background-color: var(--bg-input);
  background-position: center;
  background-repeat: no-repeat;
  background-size: 10px 10px;
  cursor: pointer;
  vertical-align: middle;
  position: relative;
  transition:
    background-color 0.18s ease,
    border-color 0.18s ease,
    box-shadow 0.18s ease;
  margin: 0;
}

input[type="checkbox"]:hover {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(120, 217, 236, 0.12);
}

input[type="checkbox"]:checked {
  background-color: var(--color-primary);
  border-color: var(--color-primary);
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%230E1626' stroke-width='3.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='20 6 9 17 4 12'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: center;
}

[data-theme="light"] input[type="checkbox"]:checked {
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%23FFFFFF' stroke-width='3.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='20 6 9 17 4 12'/%3E%3C/svg%3E");
}

.modal-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(120, 217, 236, 0.15);
}

.modal-textarea {
  height: 70px;
  padding: 6px 8px;
  resize: vertical;
  font-family: Consolas, monospace;
}

.traffic-input-box {
  display: flex;
  gap: 8px;
  align-items: center;
  width: 100%;
}
.traffic-input-box .flex-1 {
  flex: 1;
  min-width: 0;
}
.traffic-input-box .w-24 {
  width: 78px;
  flex-shrink: 0;
}

.checkbox-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  color: var(--text-main);
  cursor: pointer;
  white-space: nowrap;
  user-select: none;
  flex-shrink: 0;
}

.prop-row {
  display: flex;
  justify-content: space-between;
  padding: 4px 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  font-size: 12px;
}

.prop-row .k { color: #8E9099; }
.prop-row .v { color: #E2E2E9; font-weight: 500; }

.chmod-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.chmod-table th, .chmod-table td {
  padding: 8px;
  text-align: center;
  border: 1px solid #2B2F3D;
}

.chmod-table th { background: #222631; color: #C4C6D0; }

.modal-footer {
  height: 48px;
  padding: 0 16px;
  background: var(--bg-card);
  border-top: 1px solid var(--border-card);
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 10px;
}

.btn-cancel {
  height: 28px;
  padding: 0 14px;
  background: #7D292E;
  border: 1px solid #8C2F35;
  border-radius: var(--radius-btn);
  color: #FFFFFF;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
}

.btn-cancel:hover {
  background: #8E3036;
  border-color: #A33A42;
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(125, 41, 46, 0.3);
}

.btn-cancel:active {
  background: #682024;
  border-color: #682024;
  transform: translateY(0);
}

[data-theme="light"] .btn-cancel {
  background: #B83A41;
  border: 1px solid #A82E35;
  color: #FFFFFF;
  box-shadow: 0 1px 3px rgba(184, 58, 65, 0.15);
}

[data-theme="light"] .btn-cancel:hover {
  background: #C8464E;
  border-color: #B83A41;
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(184, 58, 65, 0.25);
}

.btn-confirm {
  height: 28px;
  padding: 0 16px;
  background: var(--btn-primary-bg);
  border: 1px solid var(--btn-primary-bg);
  border-radius: var(--radius-btn);
  color: var(--btn-primary-text);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
}

.btn-confirm:hover {
  background: var(--btn-primary-hover);
  border-color: var(--btn-primary-hover);
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
}

.btn-confirm:active {
  background: var(--color-primary-active);
  transform: translateY(1px);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
}

/* 流量统计弹窗：立即获取服务器信息 */
.traffic-sync-row {
  display: flex;
  justify-content: center;
  padding-top: 2px;
}

.traffic-sync-btn {
  height: 32px;
  min-width: 190px;
  padding: 0 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  background: var(--btn-primary-bg);
  border: 1px solid var(--btn-primary-bg);
  border-radius: var(--radius-btn);
  color: var(--btn-primary-text);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
}

.traffic-sync-btn:hover:not(:disabled) {
  background: var(--btn-primary-hover);
  border-color: var(--btn-primary-hover);
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
}

.traffic-sync-btn:active:not(:disabled) {
  transform: translateY(0);
}

.traffic-sync-btn:disabled {
  opacity: 0.62;
  cursor: wait;
  transform: none;
}

.traffic-sync-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid currentColor;
  border-top-color: transparent;
  border-radius: 50%;
  animation: traffic-sync-spin 0.75s linear infinite;
  box-sizing: border-box;
}

@keyframes traffic-sync-spin {
  to { transform: rotate(360deg); }
}

.traffic-sync-status {
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-sub);
  text-align: center;
  word-break: break-word;
}

.traffic-sync-status.is-error {
  color: #F28B82;
}

[data-theme="light"] .traffic-sync-status.is-error {
  color: #B3261E;
}

/* 偏好、备份与导入对话框扩展样式 */
.mgmt-actions-row { display: flex; align-items: center; gap: 8px; }
.btn-sec-act.flex-1 { flex: 1; text-align: center; }

.backup-section-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-primary);
  margin-top: 4px;
}
.backup-btn-row {
  display: flex;
  gap: 8px;
}
.backup-divider {
  height: 1px;
  background: var(--border-card);
  margin: 6px 0;
}
.backup-status-text {
  padding: 8px;
  background: var(--bg-capsule);
  border-radius: var(--radius-btn);
  font-size: 11px;
  color: #48BB78;
  border: 1px solid rgba(72, 187, 120, 0.25);
  line-height: 1.4;
}

.import-list {
  max-height: 260px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 4px;
}
.import-item {
  padding: 8px 10px;
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  border-radius: var(--radius-btn);
}
.import-hname { font-weight: 500; color: var(--text-main); }
.import-haddr { font-size: 11px; color: var(--text-sub); }

.search-clear {
  position: absolute;
  right: 18px;
  font-size: 11px;
  color: var(--text-sub);
  cursor: pointer;
}
.search-clear:hover { color: var(--text-main); }

/* =========================================================================
   1:1 对标 ssh8.py TerminalSettingsDialog MD3 偏好设置专用表单样式
   ========================================================================= */
.settings-modal-card {
  width: 490px;
  max-width: 95vw;
}

.settings-form-body {
  padding: 18px 24px;
  gap: 12px;
}

.settings-form-row {
  display: flex;
  align-items: center;
  min-height: 32px;
  gap: 12px;
}

.settings-form-label {
  width: 100px;
  min-width: 100px;
  font-size: 12px;
  color: var(--text-main);
  font-weight: 500;
  text-align: right;
  flex-shrink: 0;
  user-select: none;
}

.settings-form-field {
  flex: 1;
  display: flex;
  align-items: center;
}

.settings-form-field.flex-row-gap {
  gap: 10px;
}

/* 配置加密行：按钮固定宽度不换行，说明文字占满剩余空间 */
.config-encryption-field {
  flex-wrap: nowrap;
}

.config-encryption-btn {
  flex: 0 0 auto;
  white-space: nowrap;
  height: 28px;
  padding: 0 12px;
}

.config-encryption-hint {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  color: var(--text-sub);
  line-height: 1.4;
}

.selected-font-preview {
  flex: 1;
  font-size: 12px;
  color: var(--color-primary);
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  padding: 5px 10px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
}

.w-full {
  width: 100%;
}

/* 1:1 对标 ssh8.py MD3StepSpinBox & MD3BlinkFreqSpinBox */
.md3-step-spinbox {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.md3-step-spinbox.disabled {
  opacity: 0.45;
  pointer-events: none;
}

.spin-btn {
  width: 26px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-capsule);
  color: var(--color-primary);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  padding: 0;
  transition: all 0.15s ease;
}

.spin-btn:hover:not(:disabled) {
  background: var(--bg-capsule-hover);
  border-color: var(--color-primary);
}

.spin-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.spin-input {
  width: 56px;
  height: 26px;
  text-align: center;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: 6px;
  color: var(--text-main);
  font-size: 12px;
  font-weight: 500;
  outline: none;
  padding: 0;
}

/* 消除浏览器原生丑陋的上下双三角加减按钮 (针对所有 number 输入框) */
input[type="number"]::-webkit-inner-spin-button,
input[type="number"]::-webkit-outer-spin-button {
  -webkit-appearance: none;
  margin: 0;
}

input[type="number"] {
  -moz-appearance: textfield;
}

/* 针对独立 number 输入框（如时间重置框、端口框），赋予符合 QShell 视觉语言的自绘双向调节微指示标 */
input.modal-input[type="number"]:not(.spin-input) {
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='12' viewBox='0 0 10 16' fill='none'%3E%3Cpath d='M5 1L9 6H1L5 1Z' fill='%2378D9EC' opacity='0.7'/%3E%3Cpath d='M5 15L1 10H9L5 15Z' fill='%2378D9EC' opacity='0.7'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 6px center;
  padding-right: 18px !important;
}

[data-theme="light"] input.modal-input[type="number"]:not(.spin-input) {
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='12' viewBox='0 0 10 16' fill='none'%3E%3Cpath d='M5 1L9 6H1L5 1Z' fill='%231A56DB' opacity='0.7'/%3E%3Cpath d='M5 15L1 10H9L5 15Z' fill='%231A56DB' opacity='0.7'/%3E%3C/svg%3E");
}

.spin-input.spin-input-wide {
  width: 66px;
}

.spin-input:focus {
  border-color: var(--color-primary);
}

/* 1:1 对标 MD3 复选框 */
.md3-checkbox-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-main);
  cursor: pointer;
  user-select: none;
}

.md3-check-native {
  width: 15px;
  height: 15px;
  accent-color: var(--color-primary);
  cursor: pointer;
}

/* “使用系统 SSH Agent”复选框行 */
.agent-checkbox-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 2px;
  font-size: 12px;
  color: var(--text-main);
  cursor: pointer;
  user-select: none;
}

/* SSH 密钥生成结果 */
.keygen-result {
  margin-top: 10px;
  padding: 8px;
  background: var(--bg-capsule);
  border-radius: 6px;
  font-size: 11px;
}

.keygen-line {
  color: var(--text-sub);
  margin-bottom: 3px;
  word-break: break-all;
}

.keygen-pubkey {
  margin-top: 6px;
  padding: 6px;
  background: var(--bg-card);
  border-radius: 4px;
  color: var(--text-main);
  font-family: Consolas, monospace;
  font-size: 10px;
  word-break: break-all;
  max-height: 90px;
  overflow: auto;
}

/* 闪烁行排布 */
.blink-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.blink-speed-label {
  font-size: 12px;
  color: var(--text-sub);
}

/* 1:1 对标 ssh8.py 终端背景预设切换按钮 */
.bg-preset-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
}

.btn-bg-preset {
  flex: 1;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  border-radius: 8px;
  cursor: pointer;
  background: transparent;
  border: 1px solid var(--border-input);
  color: var(--text-sub);
  transition: all 0.18s ease;
}

.btn-bg-preset.active {
  background: var(--color-primary) !important;
  color: var(--color-on-primary) !important;
  border-color: var(--color-primary) !important;
  font-weight: 500;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.15);
}

.btn-bg-preset:hover:not(.active) {
  background: var(--bg-capsule-hover);
  color: var(--text-main);
}

.bg-image-settings {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  min-width: 0;
}

.bg-image-path {
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: var(--text-sub);
}

.bg-image-hint {
  font-size: 11px;
  color: var(--text-sub);
}

.bg-image-dim-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.bg-image-dim-slider {
  flex: 1;
  accent-color: var(--color-primary);
}

.bg-image-dim-value {
  width: 38px;
  text-align: right;
  font-size: 11px;
  color: var(--text-sub);
  font-variant-numeric: tabular-nums;
}

/* 1:1 对标 ssh8.py color_btn_box 调色块 */
.color-palette-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.accent-picker-row {
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
}

.accent-color-well {
  width: 28px;
  height: 28px;
  border-radius: 6px;
  overflow: hidden;
  border: 2px solid var(--border-input);
  cursor: pointer;
  display: inline-flex;
}

.accent-color-well input[type="color"] {
  width: 40px;
  height: 40px;
  margin: -6px;
  border: 0;
  padding: 0;
  cursor: pointer;
  background: transparent;
}

.btn-accent-reset {
  height: 26px;
  padding: 0 10px;
  border-radius: var(--radius-btn);
  border: 1px solid var(--border-input);
  background: var(--bg-capsule);
  color: var(--text-main);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
}

.btn-accent-reset:hover {
  background: var(--bg-capsule-hover);
  border-color: var(--color-primary);
}

.color-swatch-btn {
  width: 28px;
  height: 28px;
  border-radius: 6px;
  border: 2px solid transparent;
  cursor: pointer;
  transition: transform 0.15s ease, border-color 0.15s ease;
  padding: 0;
}

.color-swatch-btn.active {
  border-color: var(--color-primary) !important;
  transform: scale(1.1);
  box-shadow: 0 0 8px rgba(0, 0, 0, 0.25);
}

.color-swatch-btn:hover:not(.active) {
  transform: scale(1.08);
}

/* 1:1 对标 ssh8.py RainbowCustomColorButton */
.rainbow-custom-color-btn {
  width: 28px;
  height: 28px;
  border-radius: 6px;
  border: 2px solid var(--border-input);
  background: conic-gradient(from 0deg, #EA4335, #FBBC05, #34A853, #4285F4, #EA4335);
  cursor: pointer;
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: border-color 0.15s ease, transform 0.15s ease;
}

.rainbow-custom-color-btn:hover {
  border-color: var(--color-primary);
  transform: scale(1.08);
}

.rainbow-center-dot {
  width: 10px;
  height: 10px;
  background: #FFFFFF;
  border-radius: 50%;
  box-shadow: 0 0 2px rgba(0, 0, 0, 0.4);
}

.hidden-color-picker {
  position: absolute;
  opacity: 0;
  width: 0;
  height: 0;
  pointer-events: none;
}

/* 1:1 对标 ssh8.py btn_backup_center */
.backup-entry-row {
  margin-top: 6px;
  padding-top: 8px;
  border-top: 1px dashed var(--border-card);
  width: 100%;
}

.btn-backup-center {
  width: 100%;
  height: 32px;
  background: var(--color-primary);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-btn);
  color: var(--color-on-primary);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
  transition: all 0.18s cubic-bezier(0.4, 0, 0.2, 1);
}

.btn-backup-center:hover {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
  transform: translateY(-1px);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
}

.btn-backup-center:active {
  background: var(--color-primary-active);
  transform: translateY(0);
}

.sftp-quick-select {
  height: 26px;
  padding: 0 24px 0 8px !important;
  font-size: 11px;
}

/* 预警徽标与下拉样式 */
.nav-pill.alert-pill {
  color: #F28B82;
  background: rgba(242, 139, 130, 0.12);
  border-color: rgba(242, 139, 130, 0.3);
}
/* 预警徽标与下拉样式 (适配深浅主题 1:1 对标 ssh8.py) */
.alert-popover {
  width: 360px !important;
  max-width: 90vw;
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
}

.alert-head {
  padding: 10px 14px !important;
  border-top-left-radius: calc(var(--radius-card) - 1px);
  border-top-right-radius: calc(var(--radius-card) - 1px);
  background: var(--bg-sub-card) !important;
  border-bottom: 1px solid var(--border-card) !important;
}

.alert-scroll-list {
  border-bottom-left-radius: calc(var(--radius-card) - 1px);
  border-bottom-right-radius: calc(var(--radius-card) - 1px);
}

.alert-head-title {
  font-size: 13px !important;
  font-weight: 600 !important;
  color: var(--text-title) !important;
}

.alert-head-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}

.btn-popover-ignore {
  height: 24px;
  padding: 0 10px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-on-primary);
  background-color: var(--color-primary);
  border: 1px solid var(--color-primary);
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.btn-popover-ignore:hover {
  background-color: var(--color-primary-hover);
}

.alert-scroll-list {
  max-height: 320px !important;
  padding: 10px 12px !important;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.alert-empty-state {
  text-align: center;
  padding: 38px 12px;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-sub);
  user-select: none;
}

.alert-card-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  border-radius: 9px;
  cursor: pointer;
  transition: all 0.15s ease;
  user-select: none;
}

[data-theme="light"] .alert-card-item {
  background: #FAFAFC;
  border: 1px solid #D2D8E4;
}

.alert-card-item:hover {
  background: var(--bg-capsule-hover);
  border-color: var(--color-primary);
}

[data-theme="light"] .alert-card-item:hover {
  background: #EDEFF4;
  border-color: var(--color-primary);
}

.alert-item-dot-col {
  display: flex;
  padding-top: 5px;
  flex-shrink: 0;
}

.alert-indicator-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
}

.alert-indicator-dot.traffic {
  background-color: #B3261E;
  box-shadow: 0 0 6px rgba(179, 38, 30, 0.4);
}

.alert-indicator-dot.expire {
  background-color: #D97706;
  box-shadow: 0 0 6px rgba(217, 119, 6, 0.4);
}

.alert-item-content {
  flex: 1;
  min-width: 0;
}

.alert-item-msg {
  font-size: 12px;
  line-height: 1.45;
  font-weight: 500;
  color: var(--text-main);
  word-break: break-word;
}

.alert-item-hname {
  color: var(--text-title);
  margin-right: 4px;
}

/* SFTP 拖放上传悬浮提示遮罩 */
.sftp-file-list.drag-over {
  border: 2px dashed #78D9EC;
  background: rgba(120, 217, 236, 0.05);
}
.sftp-drop-overlay {
  padding: 12px;
  text-align: center;
  font-size: 11px;
  font-weight: 600;
  color: #78D9EC;
  background: rgba(120, 217, 236, 0.1);
  border-radius: 6px;
  margin-bottom: 8px;
}

.flex-center {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.tree-host-icon {
  color: #A8C7FA;
}

/* =========================================================================
   终端 xterm.js 6 自绘滚动条 (SmoothScrollableElement) 胶囊化
   xterm 6 不再使用原生 ::-webkit-scrollbar，而是渲染
   .xterm-scrollable-element > .scrollbar.vertical > .slider，
   这里强制其与 SFTP 栏滚动条 1:1 一致：
   8px 宽、透明轨道、#43474E 胶囊滑块、悬停/拖拽为主色。
   ========================================================================= */
.center-terminal-card .xterm .xterm-scrollable-element > .scrollbar.vertical,
div.terminal-mount-box .xterm .xterm-scrollable-element > .scrollbar.vertical {
  width: 8px !important;
  right: 0 !important;
  background: transparent !important;
}

.center-terminal-card .xterm .xterm-scrollable-element > .scrollbar.horizontal,
div.terminal-mount-box .xterm .xterm-scrollable-element > .scrollbar.horizontal {
  height: 8px !important;
  bottom: 0 !important;
  background: transparent !important;
}

.center-terminal-card .xterm .xterm-scrollable-element > .scrollbar.vertical > .slider,
div.terminal-mount-box .xterm .xterm-scrollable-element > .scrollbar.vertical > .slider {
  width: 8px !important;
  left: 0 !important;
  min-height: 24px !important;
  border: none !important;
  border-radius: 9999px !important;
  background-color: var(--scrollbar-thumb) !important;
}

.center-terminal-card .xterm .xterm-scrollable-element > .scrollbar.horizontal > .slider,
div.terminal-mount-box .xterm .xterm-scrollable-element > .scrollbar.horizontal > .slider {
  height: 8px !important;
  bottom: 0 !important;
  min-width: 24px !important;
  border: none !important;
  border-radius: 9999px !important;
  background-color: var(--scrollbar-thumb) !important;
}

.center-terminal-card .xterm .xterm-scrollable-element > .scrollbar > .slider:hover,
div.terminal-mount-box .xterm .xterm-scrollable-element > .scrollbar > .slider:hover,
.center-terminal-card .xterm .xterm-scrollable-element > .scrollbar > .slider.active,
div.terminal-mount-box .xterm .xterm-scrollable-element > .scrollbar > .slider.active {
  background-color: var(--scrollbar-thumb-hover) !important;
}

/* 字体选择专用弹窗样式 */
.font-selector-modal-card {
  width: 520px;
  max-width: 95vw;
}

.font-selector-body {
  padding: 14px 18px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.font-selector-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 4px;
}

.font-loading-tip {
  font-size: 11px;
  color: var(--color-primary);
}

.font-list-scrollable {
  max-height: 380px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-right: 4px;
}

.font-group-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-sub);
  margin-top: 6px;
  margin-bottom: 2px;
  padding-left: 2px;
}

.font-item-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-radius: var(--radius-btn);
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  cursor: pointer;
  transition: all 0.15s ease;
}

.font-item-row:hover {
  background: var(--bg-capsule-hover);
  border-color: var(--color-primary);
}

.font-item-row.active {
  background: rgba(120, 217, 236, 0.12);
  border-color: var(--color-primary);
}

[data-theme="light"] .font-item-row.active {
  background: rgba(26, 86, 219, 0.1);
  border-color: var(--color-primary);
}

.font-item-left {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.font-item-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
}

.font-item-preview {
  font-size: 11px;
  color: var(--text-sub);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.font-check-mark {
  color: var(--color-primary);
  font-size: 14px;
  font-weight: 500;
  padding-left: 8px;
}

:root {
  --metric-cpu-from: #3B82F6;
  --metric-cpu-to: #22D3EE;
  --metric-cpu-glow: rgba(34, 211, 238, 0.24);
  --metric-mem-from: #059669;
  --metric-mem-to: #2DD4BF;
  --metric-mem-glow: rgba(45, 212, 191, 0.22);
  --metric-disk-from: #D97706;
  --metric-disk-to: #FB923C;
  --metric-disk-glow: rgba(251, 146, 60, 0.22);
}

[data-theme="light"] {
  --metric-cpu-from: #1D4ED8;
  --metric-cpu-to: #0891B2;
  --metric-cpu-glow: rgba(8, 145, 178, 0.16);
  --metric-mem-from: #047857;
  --metric-mem-to: #0D9488;
  --metric-mem-glow: rgba(13, 148, 136, 0.16);
  --metric-disk-from: #B45309;
  --metric-disk-to: #EA580C;
  --metric-disk-glow: rgba(234, 88, 12, 0.16);
}

/* =========================================================================
   统一按钮配色：主操作跟随强调色，取消/危险统一红色语义
   ========================================================================= */
:root {
  --danger-bg: #8C2A30;
  --danger-hover: #A3343C;
  --danger-active: #722026;
  --danger-border: #A33840;
  --danger-text: #FFFFFF;
  --danger-soft-bg: rgba(196, 76, 84, 0.12);
  --danger-soft-border: rgba(196, 76, 84, 0.35);
}

[data-theme="light"] {
  --danger-bg: #B83A41;
  --danger-hover: #C94B52;
  --danger-active: #9E2F36;
  --danger-border: #A82E35;
  --danger-text: #FFFFFF;
  --danger-soft-bg: rgba(184, 58, 65, 0.1);
  --danger-soft-border: rgba(184, 58, 65, 0.3);
}

/* 主操作按钮统一使用强调色 */
.btn-card-act:not(.btn-del):not(.btn-danger),
.btn-primary,
.btn-sec-act:not(.btn-browse),
.btn-add-host,
.btn-confirm,
.btn-backup-center,
.btn-mgmt-pill,
.btn-sftp-action,
.btn-sftp-footer,
.btn-switch-white-pill:not(.btn-danger),
.btn-popover-clear,
.btn-bottom-add-pill,
.btn-bg-preset.active,
.btn-col-choice.active {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: var(--color-on-primary);
}

/* 取消、删除、断开等危险语义统一红色 */
.btn-cancel,
.btn-danger,
.btn-del,
.btn-danger-disconnect,
.btn-card-act.btn-del,
.btn-card-act.btn-danger {
  background: var(--danger-bg) !important;
  border-color: var(--danger-border) !important;
  color: var(--danger-text) !important;
}

.btn-cancel:hover,
.btn-danger:hover,
.btn-del:hover,
.btn-danger-disconnect:hover,
.btn-card-act.btn-del:hover,
.btn-card-act.btn-danger:hover {
  background: var(--danger-hover) !important;
  border-color: var(--danger-hover) !important;
}

.btn-cancel:active,
.btn-danger:active,
.btn-del:active,
.btn-danger-disconnect:active,
.btn-card-act.btn-del:active,
.btn-card-act.btn-danger:active {
  background: var(--danger-active) !important;
  border-color: var(--danger-active) !important;
  transform: translateY(1px);
}



/* =========================================================================
   全局字重层级：正文 400 / 控件与列表 500 / 标题与关键值 600
   ========================================================================= */
body,
.app-root {
  font-weight: 400;
}

.g-name,
.g-val,
.metric-model,
.metric-detail,
.metric-chip,
.settings-form-label,
.f-name,
.f-size,
.f-perm,
.file-item,
.sftp-table-header,
.col-name,
.col-size,
.col-perm,
.group-name,
.host-name,
.host-item,
.term-tab,
.tab-name,
.memo-name,
.task-name,
.task-badge,
.status-tag,
.net-text .dir,
.net-text .spd,
.unified-traffic-pct,
.import-hname,
.prop-row .v {
  font-weight: 500 !important;
}

.card-header-title,
.right-card-title,
.modal-title,
.empty-title,
.mgmt-title,
.card-name {
  font-weight: 500 !important;
}

.logo-title {
  font-weight: 600 !important;
}

.local-picker-modal {
  width: 720px;
  max-width: calc(100vw - 48px);
  height: min(620px, 78vh);
  display: flex;
  flex-direction: column;
}

.local-picker-body {
  display: flex;
  flex-direction: row;
  gap: 0;
  padding: 12px 14px;
  flex: 1;
  min-height: 0;
  max-height: none;
}

.local-picker-sidebar {
  width: 132px;
  flex: 0 0 132px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-right: 10px;
  margin-right: 12px;
  border-right: 1px solid var(--border-card);
}

.local-picker-side-item {
  width: 100%;
  height: 28px;
  padding: 0 7px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text-main);
  display: flex;
  align-items: center;
  gap: 7px;
  font-family: inherit;
  font-size: 10px;
  font-weight: 500;
  cursor: pointer;
  text-align: left;
}

.local-picker-side-item:hover {
  background: var(--bg-capsule-hover);
}

.local-picker-side-item.active {
  background: var(--bg-capsule);
  color: var(--color-primary);
}

.local-picker-side-divider {
  height: 1px;
  background: var(--border-card);
  margin: 5px 4px;
}

.local-picker-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.local-picker-network-hint {
  padding: 30px 12px;
  text-align: center;
  color: var(--text-sub);
  font-size: 11px;
}

.local-picker-network-example {
  margin-top: 8px;
  color: var(--text-main);
  font-family: Consolas, monospace;
  font-size: 12px;
}

.local-picker-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
}

.local-picker-path {
  flex: 1;
  min-width: 0;
  height: 30px;
  padding: 0 9px;
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  background: var(--bg-input);
  color: var(--text-main);
  font-size: 11px;
  outline: none;
}

.local-picker-path:focus {
  border-color: var(--color-primary);
}

.local-picker-roots {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}

.local-picker-root {
  height: 24px;
  padding: 0 9px;
  border: 1px solid var(--border-capsule);
  border-radius: 4px;
  background: var(--bg-capsule);
  color: var(--text-sub);
  font-size: 10px;
  font-weight: 500;
  cursor: pointer;
}

.local-picker-root:hover {
  border-color: var(--color-primary);
  color: var(--text-main);
}

.local-picker-error {
  padding: 7px 9px;
  border: 1px solid var(--danger-soft-border);
  border-radius: var(--radius-btn);
  background: var(--danger-soft-bg);
  color: #FCA5A5;
  font-size: 11px;
}

.local-picker-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  border: 1px solid var(--border-card);
  border-radius: var(--radius-btn);
  background: var(--bg-sub-card);
  padding: 4px;
}

.local-picker-thumb {
  width: 26px;
  height: 26px;
  flex: 0 0 26px;
  object-fit: cover;
  border-radius: 4px;
  background: rgba(0, 0, 0, 0.08);
}

.local-picker-item {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 8px;
  border-radius: 4px;
  color: var(--text-main);
  font-size: 11px;  cursor: pointer;
  user-select: none;
}

.local-picker-item:hover {
  background: var(--bg-capsule-hover);
}

.local-picker-item.selected {
  background: var(--bg-capsule);
  color: var(--color-primary);
}

.local-picker-item.is-dir {
  color: var(--text-title);
}

.local-picker-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.local-picker-size {
  flex: 0 0 auto;
  color: var(--text-sub);
  font-family: Consolas, monospace;
  font-size: 10px;
}

.local-picker-empty {
  padding: 28px 12px;
  text-align: center;
  color: var(--text-sub);
  font-size: 11px;
}

.local-picker-save-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--border-card);
}

.local-picker-save-row label {
  flex: 0 0 auto;
  font-size: 12px;
  color: var(--text-sub);
}

.local-picker-save-row .local-picker-path {
  flex: 1;
}

.local-picker-footer {
  display: flex;
  align-items: center;
  gap: 8px;
}

.local-picker-selected {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--text-sub);
  font-size: 10px;
}


.confirm-modal-items-list {
  margin-top: 12px;
  max-height: 180px;
  overflow-y: auto;
  background: var(--bg-sub-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-btn);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.confirm-modal-item-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-main);
  line-height: 1.4;
  word-break: break-all;
}

.confirm-item-bullet {
  color: var(--danger-border);
  font-weight: bold;
}

.confirm-item-name {
  flex: 1;
  font-family: var(--font-mono, Consolas, monospace);
  font-size: 11px;
}

/* WinUI 3 风格 Telnet 安全性警告 InfoBar */
.infobar-warning {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  margin: 6px 0 12px 0;
  border-radius: var(--radius-sm, 6px);
  background: rgba(234, 179, 8, 0.12);
  border: 1px solid rgba(234, 179, 8, 0.35);
  color: #d97706;
  font-size: 12px;
  line-height: 1.45;
}

.infobar-icon {
  font-size: 14px;
  flex-shrink: 0;
}

.infobar-text {
  flex: 1;
}

/* 串口高级通信参数栅格布局 */
.serial-advanced-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
  padding: 10px 12px;
  margin: 6px 0 12px 0;
  background: var(--bg-hover, rgba(0, 0, 0, 0.03));
  border: 1px dashed var(--border-color, rgba(0, 0, 0, 0.1));
  border-radius: var(--radius-sm, 6px);
}

.serial-advanced-grid .field-col {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.serial-advanced-grid label {
  font-size: 11px;
  font-weight: 500;
  color: var(--text-sec, #666);
}
</style>
