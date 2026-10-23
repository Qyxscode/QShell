<template>
  <div class="custom-num-stepper" :class="[customClass, { disabled }]">
    <input
      type="text"
      inputmode="numeric"
      :value="formattedValue"
      @input="handleInput"
      @keydown.up.prevent="stepUp"
      @keydown.down.prevent="stepDown"
      class="stepper-input"
      :disabled="disabled"
    />
    <div class="stepper-controls">
      <button
        type="button"
        class="stepper-btn up"
        :disabled="disabled || (max !== undefined && modelValue >= max)"
        @click="stepUp"
        title="增加"
      >
        <svg width="8" height="6" viewBox="0 0 10 6" fill="none">
          <path d="M1 5L5 1L9 5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
      <button
        type="button"
        class="stepper-btn down"
        :disabled="disabled || (min !== undefined && modelValue <= min)"
        @click="stepDown"
        title="减少"
      >
        <svg width="8" height="6" viewBox="0 0 10 6" fill="none">
          <path d="M1 1L5 5L9 1" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{
  modelValue: number;
  min?: number;
  max?: number;
  step?: number;
  padZero?: boolean;
  customClass?: string;
  disabled?: boolean;
}>(), {
  min: 0,
  step: 1,
  padZero: false,
  customClass: "",
  disabled: false,
});

const emit = defineEmits<{
  (e: "update:modelValue", val: number): void;
}>();

const formattedValue = computed(() => {
  if (props.padZero && typeof props.modelValue === "number") {
    return String(props.modelValue).padStart(2, "0");
  }
  return String(props.modelValue ?? 0);
});

function stepUp() {
  if (props.disabled) return;
  const next = (props.modelValue || 0) + props.step;
  if (props.max !== undefined && next > props.max) return;
  emit("update:modelValue", next);
}

function stepDown() {
  if (props.disabled) return;
  const next = (props.modelValue || 0) - props.step;
  if (props.min !== undefined && next < props.min) return;
  emit("update:modelValue", next);
}

function handleInput(e: Event) {
  const target = e.target as HTMLInputElement;
  let parsed = parseInt(target.value.replace(/[^\d]/g, ""), 10);
  if (isNaN(parsed)) parsed = props.min ?? 0;
  if (props.min !== undefined && parsed < props.min) parsed = props.min;
  if (props.max !== undefined && parsed > props.max) parsed = props.max;
  emit("update:modelValue", parsed);
}
</script>

<style scoped>
.custom-num-stepper {
  display: inline-flex;
  align-items: center;
  position: relative;
  background: var(--bg-input);
  border: 1px solid var(--border-input);
  border-radius: var(--radius-btn);
  height: 28px;
  box-sizing: border-box;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.custom-num-stepper:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(120, 217, 236, 0.15);
}

.custom-num-stepper.disabled {
  opacity: 0.5;
  pointer-events: none;
}

.stepper-input {
  width: 100%;
  height: 100%;
  border: none;
  background: transparent;
  color: var(--text-main);
  font-size: 12px;
  font-weight: 600;
  text-align: center;
  outline: none;
  padding: 0 18px 0 6px;
  font-family: inherit;
  box-sizing: border-box;
}

.stepper-controls {
  position: absolute;
  right: 2px;
  top: 2px;
  bottom: 2px;
  width: 16px;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 1px;
}

.stepper-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-primary);
  border-radius: 2px;
  cursor: pointer;
  padding: 0;
  transition: all 0.15s ease;
}

.stepper-btn:hover:not(:disabled) {
  background: var(--bg-capsule-hover);
  color: var(--color-primary-hover);
}

.stepper-btn:disabled {
  opacity: 0.3;
  cursor: not-allowed;
}

.custom-num-stepper.reset-digit-stepper {
  width: 58px;
}

.custom-num-stepper.reset-day-stepper {
  width: 68px;
}
</style>
