<template>
  <div class="custom-select-box" :class="[customClass, { open: isOpen, disabled }]" ref="selectBoxRef">
    <div class="select-trigger" @click="toggleOpen">
      <span class="selected-content">
        <span
          v-if="selectedOption?.icon"
          :class="['cursor-glyph', selectedOption.icon]"
          aria-hidden="true"
        ></span>
        <span class="selected-text">{{ selectedLabel }}</span>
      </span>
      <svg class="select-chevron" :class="{ rotated: isOpen }" width="12" height="12" viewBox="0 0 24 24" fill="none">
        <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    </div>

    <transition name="select-popover-anim">
      <div v-if="isOpen" class="select-dropdown">
        <div
          v-for="opt in options"
          :key="opt.value"
          :class="['select-option', { active: opt.value === modelValue }]"
          @click="selectOption(opt.value)"
        >
          <span class="option-main">
            <span
              v-if="opt.icon"
              :class="['cursor-glyph', opt.icon]"
              aria-hidden="true"
            ></span>
            <span>{{ opt.label }}</span>
          </span>
          <span v-if="opt.value === modelValue" class="opt-check">✓</span>
        </div>
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";

const props = withDefaults(defineProps<{
  modelValue: any;
  options: { label: string; value: any; icon?: string }[];
  customClass?: string;
  disabled?: boolean;
}>(), {
  customClass: "",
  disabled: false,
});

const emit = defineEmits<{
  (e: "update:modelValue", val: any): void;
  (e: "change", val: any): void;
}>();

const isOpen = ref(false);
const selectBoxRef = ref<HTMLElement | null>(null);

const selectedLabel = computed(() => {
  const found = props.options.find(o => o.value === props.modelValue);
  return found ? found.label : (props.modelValue ?? "");
});

const selectedOption = computed(() => {
  return props.options.find(o => o.value === props.modelValue) || null;
});

function toggleOpen() {
  if (props.disabled) return;
  const willOpen = !isOpen.value;
  window.dispatchEvent(new CustomEvent("qshell-close-selects"));
  isOpen.value = willOpen;
}

function handleCloseSelects() {
  isOpen.value = false;
}

function selectOption(val: any) {
  emit("update:modelValue", val);
  emit("change", val);
  isOpen.value = false;
}

function handleOutsideClick(e: MouseEvent) {
  if (selectBoxRef.value && !selectBoxRef.value.contains(e.target as Node)) {
    isOpen.value = false;
  }
}

onMounted(() => {
  window.addEventListener("click", handleOutsideClick);
  window.addEventListener("qshell-close-selects", handleCloseSelects);
});

onUnmounted(() => {
  window.removeEventListener("click", handleOutsideClick);
  window.removeEventListener("qshell-close-selects", handleCloseSelects);
});
</script>

<style scoped>
.custom-select-box {
  position: relative;
  display: inline-block;
  user-select: none;
  font-family: inherit;
  font-size: 12px;
}

.select-trigger {
  height: 32px;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  padding: 0 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  color: var(--text-main);
  cursor: pointer;
  box-sizing: border-box;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.custom-select-box.w-28 { width: 110px; }
.custom-select-box.w-24 { width: 85px; }
.custom-select-box.w-full { width: 100%; }
.custom-select-box.text-center .selected-text { text-align: center; }

.custom-select-box.open .select-trigger,
.select-trigger:hover {
  border-color: var(--color-primary);
}

.custom-select-box.open .select-trigger {
  box-shadow: 0 0 0 2px rgba(120, 217, 236, 0.15);
}

.custom-select-box.disabled {
  opacity: 0.5;
  cursor: not-allowed;
  pointer-events: none;
}

.selected-content {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 7px;
}

.selected-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
}

.option-main {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

/* Cursor preview glyphs: visually distinguish block / bar / underline. */
.cursor-glyph {
  flex: 0 0 auto;
  display: inline-block;
  position: relative;
  width: 14px;
  height: 16px;
  color: currentColor;
}

.cursor-glyph.block::before {
  content: "";
  position: absolute;
  left: 3px;
  top: 3px;
  width: 8px;
  height: 11px;
  border-radius: 1.5px;
  background: currentColor;
}

.cursor-glyph.bar::before {
  content: "";
  position: absolute;
  left: 6px;
  top: 2px;
  width: 2px;
  height: 12px;
  border-radius: 1px;
  background: currentColor;
}

.cursor-glyph.underline::before {
  content: "";
  position: absolute;
  left: 2px;
  top: 10px;
  width: 10px;
  height: 2px;
  border-radius: 1px;
  background: currentColor;
}

.select-chevron {
  flex-shrink: 0;
  color: var(--color-primary);
  transition: transform 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}

.select-chevron.rotated {
  transform: rotate(180deg);
}

.select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  min-width: 100%;
  width: max-content;
  max-width: 320px;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: 6px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  z-index: 99999;
  padding: 4px;
  max-height: 220px;
  overflow-y: auto;
}

[data-theme="light"] .select-dropdown {
  background: #FFFFFF;
  border-color: #DDE2EC;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
}

.select-option {
  padding: 6px 10px;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  color: var(--text-main);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
  font-size: 12px;
}

.select-option:hover {
  background: var(--bg-capsule-hover);
  color: var(--color-primary);
}

[data-theme="light"] .select-option:hover {
  background: #EFF6FF;
  color: #1A56DB;
}

.select-option.active {
  background: var(--color-primary);
  color: var(--color-on-primary);
  font-weight: 600;
}

.opt-check {
  font-size: 11px;
}

.select-popover-anim-enter-active,
.select-popover-anim-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.select-popover-anim-enter-from,
.select-popover-anim-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
