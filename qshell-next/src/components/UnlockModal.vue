<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{
  modelValue: boolean;
  busy: boolean;
  error: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: boolean): void;
  (e: "submit", password: string): void;
}>();

const password = ref("");

watch(
  () => props.modelValue,
  (show) => {
    if (show) password.value = "";
  }
);

function submit() {
  emit("submit", password.value);
}
</script>

<template>
  <Transition name="modal-fade">
    <div v-if="modelValue" class="modal-overlay" style="z-index: 400000;">
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">配置已加密</span>
        </div>
        <div class="modal-body">
          <p class="confirm-modal-text">请输入主密码解锁本地配置。忘记密码只能清空配置重来。</p>
          <input
            v-model="password"
            type="password"
            class="modal-input"
            placeholder="主密码"
            @keyup.enter="submit"
          />
          <div v-if="error" class="infobar-warning" style="margin-top: 8px;">
            <span class="infobar-icon">⚠️</span>
            <span class="infobar-text">{{ error }}</span>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-confirm" :disabled="busy" @click="submit">
            {{ busy ? "解锁中..." : "解锁" }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
