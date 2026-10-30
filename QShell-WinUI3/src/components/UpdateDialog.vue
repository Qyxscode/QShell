<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{
  modelValue: boolean;
  mode: "latest" | "available" | "downloading" | "error";
  version: string;
  notes: string;
  error: string;
  progress: number;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: boolean): void;
  (e: "install"): void;
}>();

function close() {
  emit("update:modelValue", false);
}

// ---- 版本号的强调色需要做对比度兜底 ----
// 直接用 --color-primary 在「暗色模式 + 灰色强调色」时会暗到看不清，
// 这里读实际计算出的颜色与弹窗背景比一次，不够亮就退回正文色。

function normalizeColor(input: string): string | null {
  const value = (input || "").trim();
  if (!value) return null;
  if (/^#[0-9a-fA-F]{6}$/.test(value)) return value;
  if (/^#[0-9a-fA-F]{3}$/.test(value)) {
    return `#${value[1]}${value[1]}${value[2]}${value[2]}${value[3]}${value[3]}`;
  }
  const rgbMatch = value.match(/^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/i);
  if (rgbMatch) {
    const toHex = (n: string) => Number(n).toString(16).padStart(2, "0");
    return `#${toHex(rgbMatch[1])}${toHex(rgbMatch[2])}${toHex(rgbMatch[3])}`;
  }
  return null;
}

function luminance(hex: string): number {
  const value = hex.replace("#", "");
  const channels = [0, 2, 4].map((i) => {
    const c = parseInt(value.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrastRatio(a: string, b: string): number {
  const la = luminance(a);
  const lb = luminance(b);
  const lighter = Math.max(la, lb);
  const darker = Math.min(la, lb);
  return (lighter + 0.05) / (darker + 0.05);
}

const versionColor = ref("var(--color-primary)");

function computeVersionColor() {
  try {
    const style = getComputedStyle(document.documentElement);
    const primary = normalizeColor(style.getPropertyValue("--color-primary"));
    const background =
      normalizeColor(style.getPropertyValue("--bg-modal")) ||
      normalizeColor(style.getPropertyValue("--bg-card"));
    if (!primary || !background) {
      versionColor.value = "var(--color-primary)";
      return;
    }
    // 4.5 是 WCAG AA 的正文对比度阈值
    versionColor.value = contrastRatio(primary, background) >= 4.5
      ? "var(--color-primary)"
      : "var(--text-main)";
  } catch {
    versionColor.value = "var(--color-primary)";
  }
}

watch(
  () => [props.modelValue, props.mode] as const,
  ([visible, mode]) => {
    if (visible && mode === "available") {
      // 等弹窗挂载、主题变量生效后再取色
      requestAnimationFrame(computeVersionColor);
    }
  },
  { immediate: true }
);
</script>

<template>
  <Transition name="modal-fade">
    <div
      v-if="modelValue"
      class="modal-overlay"
      style="z-index: 400000;"
      @mousedown.self="mode !== 'downloading' && close()"
    >
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">
            {{ mode === "available" ? "发现新版本" : mode === "latest" ? "检查更新" : mode === "downloading" ? "正在更新" : "更新失败" }}
          </span>
          <span v-if="mode !== 'downloading'" class="modal-close" @click="close">✕</span>
        </div>
        <div class="modal-body">
          <template v-if="mode === 'available'">
            <p class="confirm-modal-text">
              新版本 <b class="update-version" :style="{ color: versionColor }">v{{ version }}</b> 已发布，是否立即下载并安装？
            </p>
            <div v-if="notes" class="update-notes">{{ notes }}</div>
          </template>
          <template v-else-if="mode === 'latest'">
            <p class="confirm-modal-text">当前已是最新版本，无需更新。</p>
          </template>
          <template v-else-if="mode === 'downloading'">
            <p class="confirm-modal-text">正在下载更新，请稍候…</p>
            <div class="update-progress-track">
              <div class="update-progress-bar" :style="{ width: progress + '%' }"></div>
            </div>
            <div class="update-progress-text">{{ progress }}%</div>
          </template>
          <template v-else>
            <p class="confirm-modal-text">{{ error || "检查更新失败，请稍后再试。" }}</p>
          </template>
        </div>
        <div class="modal-footer">
          <button v-if="mode === 'available'" class="btn-cancel" @click="close">稍后</button>
          <button v-if="mode === 'available'" class="btn-confirm" @click="emit('install')">立即更新</button>
          <button v-if="mode === 'latest' || mode === 'error'" class="btn-confirm" @click="close">好的</button>
        </div>
      </div>
    </div>
  </Transition>
</template>
