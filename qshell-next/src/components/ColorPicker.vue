<template>
  <div class="cp-root">
    <button
      ref="triggerRef"
      type="button"
      class="cp-trigger"
      :style="{ backgroundColor: currentHex }"
      :title="title"
      @click.stop="toggle"
    >
      <span class="cp-trigger-shine"></span>
    </button>
  </div>

  <teleport to="body">
    <div
      v-if="open"
      ref="panelRef"
      class="cp-panel"
      :style="panelStyle"
      @pointerdown.stop
      @click.stop
    >
      <div
        ref="svRef"
        class="cp-sv"
        :style="{ background: svBackground }"
        @pointerdown="startSvDrag"
      >
        <span class="cp-sv-cursor" :style="{ left: saturation + '%', top: (100 - value) + '%' }"></span>
      </div>

      <div ref="hueRef" class="cp-hue" @pointerdown="startHueDrag">
        <span class="cp-hue-thumb" :style="{ left: (hue / 360) * 100 + '%' }"></span>
      </div>

      <div class="cp-value-row">
        <span class="cp-preview" :style="{ backgroundColor: currentHex }"></span>
        <input
          v-model="hexInput"
          class="cp-hex-input"
          maxlength="7"
          spellcheck="false"
          @change="commitHex"
          @keydown.enter="commitHex"
        />
      </div>

      <div v-if="presets.length" class="cp-presets">
        <button
          v-for="color in presets"
          :key="color"
          type="button"
          class="cp-preset"
          :class="{ active: color.toLowerCase() === currentHex.toLowerCase() }"
          :style="{ backgroundColor: color }"
          :title="color"
          @click="selectColor(color)"
        ></button>
      </div>
    </div>
  </teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from "vue";

const props = withDefaults(defineProps<{
  modelValue?: string;
  presets?: string[];
  title?: string;
}>(), {
  modelValue: "#A8C7FA",
  presets: () => [],
  title: "选择颜色",
});

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

const triggerRef = ref<HTMLElement | null>(null);
const panelRef = ref<HTMLElement | null>(null);
const svRef = ref<HTMLElement | null>(null);
const hueRef = ref<HTMLElement | null>(null);
const open = ref(false);
const hue = ref(210);
const saturation = ref(60);
const value = ref(95);
const hexInput = ref(props.modelValue);
const panelStyle = ref<Record<string, string>>({
  position: "fixed",
  left: "-9999px",
  top: "-9999px",
});
let dragging: "sv" | "hue" | null = null;

function clamp(n: number, min: number, max: number) {
  return Math.min(max, Math.max(min, n));
}

function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  let raw = String(hex || "").trim().replace("#", "");
  if (/^[0-9a-f]{3}$/i.test(raw)) {
    raw = raw.split("").map((c) => c + c).join("");
  }
  if (!/^[0-9a-f]{6}$/i.test(raw)) return null;
  const int = parseInt(raw, 16);
  return { r: (int >> 16) & 0xff, g: (int >> 8) & 0xff, b: int & 0xff };
}

function rgbToHex(r: number, g: number, b: number): string {
  return "#" + [r, g, b].map((v) => Math.round(clamp(v, 0, 255)).toString(16).padStart(2, "0")).join("").toUpperCase();
}

function rgbToHsv(r: number, g: number, b: number) {
  const rn = r / 255, gn = g / 255, bn = b / 255;
  const max = Math.max(rn, gn, bn), min = Math.min(rn, gn, bn);
  const d = max - min;
  let h = 0;
  if (d !== 0) {
    if (max === rn) h = ((gn - bn) / d) % 6;
    else if (max === gn) h = (bn - rn) / d + 2;
    else h = (rn - gn) / d + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  return { h, s: max === 0 ? 0 : (d / max) * 100, v: max * 100 };
}

function hsvToRgb(h: number, s: number, v: number) {
  const c = (v / 100) * (s / 100);
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = v / 100 - c;
  let r = 0, g = 0, b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return { r: (r + m) * 255, g: (g + m) * 255, b: (b + m) * 255 };
}

const currentHex = computed(() => {
  const { r, g, b } = hsvToRgb(hue.value, saturation.value, value.value);
  return rgbToHex(r, g, b);
});

const svBackground = computed(() =>
  `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, hsl(${hue.value} 100% 50%))`
);

function syncFromHex(hex: string) {
  const rgb = hexToRgb(hex);
  if (!rgb) return;
  const hsv = rgbToHsv(rgb.r, rgb.g, rgb.b);
  hue.value = hsv.h;
  saturation.value = hsv.s;
  value.value = hsv.v;
  hexInput.value = rgbToHex(rgb.r, rgb.g, rgb.b);
}

watch(() => props.modelValue, (val) => {
  if (!open.value) syncFromHex(val);
}, { immediate: true });

watch(currentHex, (val) => {
  hexInput.value = val;
  if (open.value) emit("update:modelValue", val);
});

function emitColor() {
  hexInput.value = currentHex.value;
  emit("update:modelValue", currentHex.value);
}

function selectColor(color: string) {
  syncFromHex(color);
  emitColor();
}

function commitHex() {
  const rgb = hexToRgb(hexInput.value);
  if (!rgb) {
    hexInput.value = currentHex.value;
    return;
  }
  hexInput.value = rgbToHex(rgb.r, rgb.g, rgb.b);
  syncFromHex(hexInput.value);
  emitColor();
}

function updatePanelPosition() {
  if (!open.value || !triggerRef.value) return;
  const anchor = triggerRef.value.getBoundingClientRect();
  const panel = panelRef.value;
  const width = panel?.offsetWidth || 224;
  const height = panel?.offsetHeight || 286;
  const margin = 8;
  let left = anchor.left;
  if (left + width > window.innerWidth - margin) left = window.innerWidth - width - margin;
  left = Math.max(margin, left);
  let top = anchor.bottom + 6;
  if (top + height > window.innerHeight - margin) top = anchor.top - height - 6;
  top = Math.max(margin, top);
  panelStyle.value = {
    position: "fixed",
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
  };
}

function toggle() {
  open.value = !open.value;
  if (open.value) {
    syncFromHex(props.modelValue);
    nextTick(() => {
      updatePanelPosition();
      requestAnimationFrame(updatePanelPosition);
    });
  }
}

function updateSvFromEvent(event: PointerEvent) {
  const el = svRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  saturation.value = clamp(((event.clientX - rect.left) / rect.width) * 100, 0, 100);
  value.value = clamp((1 - (event.clientY - rect.top) / rect.height) * 100, 0, 100);
  emitColor();
}

function updateHueFromEvent(event: PointerEvent) {
  const el = hueRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  hue.value = clamp(((event.clientX - rect.left) / rect.width) * 360, 0, 360);
  emitColor();
}

function onPointerMove(event: PointerEvent) {
  if (dragging === "sv") updateSvFromEvent(event);
  else if (dragging === "hue") updateHueFromEvent(event);
}

function stopDrag() {
  dragging = null;
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", stopDrag);
}

function startSvDrag(event: PointerEvent) {
  event.preventDefault();
  dragging = "sv";
  updateSvFromEvent(event);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", stopDrag);
}

function startHueDrag(event: PointerEvent) {
  event.preventDefault();
  dragging = "hue";
  updateHueFromEvent(event);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", stopDrag);
}

function onDocumentPointerDown(event: PointerEvent) {
  const target = event.target as Node;
  if (triggerRef.value?.contains(target) || panelRef.value?.contains(target)) return;
  open.value = false;
}

function onWindowChange() {
  if (open.value) updatePanelPosition();
}

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener("pointerdown", onDocumentPointerDown);
    window.addEventListener("resize", onWindowChange);
    window.addEventListener("scroll", onWindowChange, true);
  } else {
    document.removeEventListener("pointerdown", onDocumentPointerDown);
    window.removeEventListener("resize", onWindowChange);
    window.removeEventListener("scroll", onWindowChange, true);
  }
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  window.removeEventListener("resize", onWindowChange);
  window.removeEventListener("scroll", onWindowChange, true);
  stopDrag();
});
</script>

<style scoped>
.cp-root {
  display: inline-flex;
}

.cp-trigger {
  position: relative;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 2px solid var(--border-input);
  border-radius: 6px;
  cursor: pointer;
  overflow: hidden;
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.12);
}

.cp-trigger:hover {
  border-color: var(--color-primary);
}

.cp-trigger-shine {
  position: absolute;
  inset: 0;
  background: linear-gradient(145deg, rgba(255, 255, 255, 0.28), transparent 48%);
  pointer-events: none;
}

.cp-panel {
  width: 224px;
  padding: 10px;
  z-index: 10060;
  background: var(--bg-modal);
  border: 1px solid var(--border-card);
  border-radius: 10px;
  box-shadow: 0 12px 30px rgba(0, 0, 0, 0.32);
  user-select: none;
}

.cp-sv {
  position: relative;
  height: 126px;
  border-radius: 6px;
  cursor: crosshair;
  overflow: hidden;
  border: 1px solid var(--border-capsule);
}

.cp-sv-cursor {
  position: absolute;
  width: 12px;
  height: 12px;
  border: 2px solid #fff;
  border-radius: 50%;
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.75), 0 1px 4px rgba(0, 0, 0, 0.4);
  transform: translate(-50%, -50%);
  pointer-events: none;
}

.cp-hue {
  position: relative;
  height: 10px;
  margin-top: 10px;
  border-radius: 5px;
  background: linear-gradient(90deg, #f00, #ff0, #0f0, #0ff, #00f, #f0f, #f00);
  cursor: pointer;
}

.cp-hue-thumb {
  position: absolute;
  top: 50%;
  width: 12px;
  height: 12px;
  background: #fff;
  border: 2px solid var(--bg-modal);
  border-radius: 50%;
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.45);
  transform: translate(-50%, -50%);
  pointer-events: none;
}

.cp-value-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
}

.cp-preview {
  width: 26px;
  height: 26px;
  flex: 0 0 auto;
  border-radius: 6px;
  border: 1px solid var(--border-input);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.14);
}

.cp-hex-input {
  flex: 1;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  border-radius: 6px;
  border: 1px solid var(--border-input);
  background: var(--bg-input);
  color: var(--text-main);
  font-family: Consolas, monospace;
  font-size: 12px;
  outline: none;
}

.cp-hex-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-primary) 18%, transparent);
}

.cp-presets {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 10px;
}

.cp-preset {
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: 5px;
  border: 2px solid transparent;
  cursor: pointer;
}

.cp-preset.active {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 1px var(--bg-modal);
}

.cp-preset:hover {
  transform: scale(1.08);
}
</style>
