<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{
  modelValue: boolean;
  mode: "enable" | "disable";
  busy: boolean;
  error: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: boolean): void;
  (e: "submit", payload: { password: string; confirm: string }): void;
}>();

const password = ref("");
const confirm = ref("");

watch(
  () => props.modelValue,
  (show) => {
    if (show) {
      password.value = "";
      confirm.value = "";
    }
  }
);

function submit() {
  emit("submit", { password: password.value, confirm: confirm.value });
}
</script>

<template>
  <Transition name="modal-fade">
    <div v-if="modelValue" class="modal-overlay" style="z-index: 400000;" @mousedown.self="emit('update:modelValue', false)">
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">{{ mode === "enable" ? "开启配置加密" : "关闭配置加密" }}</span>
          <span class="modal-close" @click="emit('update:modelValue', false)">✕</span>
        </div>
        <div class="modal-body">
          <p v-if="mode === 'enable'" class="confirm-modal-text">
            设置主密码后，本地配置文件将被 AES-256-GCM 加密。忘记密码将无法恢复，只能清空配置重新开始。
          </p>
          <p v-else class="confirm-modal-text">
            输入当前主密码关闭加密，配置文件将恢复为明文保存。
          </p>
          <input
            v-model="password"
            type="password"
            class="modal-input"
            placeholder="主密码"
            @keyup.enter="submit"
          />
          <input
            v-if="mode === 'enable'"
            v-model="confirm"
            type="password"
            class="modal-input"
            style="margin-top: 8px;"
            placeholder="再次输入主密码"
            @keyup.enter="submit"
          />
          <div v-if="error" class="infobar-warning" style="margin-top: 8px;">
            <span class="infobar-icon">⚠️</span>
            <span class="infobar-text">{{ error }}</span>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="emit('update:modelValue', false)">取消</button>
          <button class="btn-confirm" :disabled="busy" @click="submit">
            {{ busy ? "处理中..." : "确定" }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
