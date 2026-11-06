<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";

/**
 * 工具面板用的下拉选择器。
 * 不用原生 <select>：Windows 上的下拉列表是系统渲染的（灰底白字），
 * 和软件外观完全不一致，这里用自定义菜单保证风格统一。
 */
const props = defineProps<{
  modelValue: number | string;
  options: { value: number | string; label: string }[];
  width?: number;
}>();

const emit = defineEmits<{ (e: "update:modelValue", v: number | string): void }>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);

const currentLabel = computed(
  () => props.options.find((o) => o.value === props.modelValue)?.label ?? String(props.modelValue),
);

function pick(value: number | string) {
  emit("update:modelValue", value);
  open.value = false;
}

function onDocClick(e: MouseEvent) {
  if (root.value && !root.value.contains(e.target as Node)) open.value = false;
}

onMounted(() => document.addEventListener("mousedown", onDocClick));
onUnmounted(() => document.removeEventListener("mousedown", onDocClick));
</script>

<template>
  <div ref="root" class="tool-select" :style="width ? { width: width + 'px' } : undefined">
    <button type="button" class="tool-select-trigger" @click="open = !open">
      <span>{{ currentLabel }}</span>
      <svg viewBox="0 0 24 24" width="10" height="10" fill="none" aria-hidden="true" class="tool-select-arrow">
        <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
    <Transition name="tool-select-fade">
      <div v-if="open" class="tool-select-menu">
        <button
          v-for="opt in options"
          :key="String(opt.value)"
          type="button"
          class="tool-select-option"
          :class="opt.value === modelValue ? 'is-active' : ''"
          @click="pick(opt.value)"
        >
          {{ opt.label }}
        </button>
      </div>
    </Transition>
  </div>
</template>
