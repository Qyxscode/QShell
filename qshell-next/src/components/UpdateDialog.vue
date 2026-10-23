<script setup lang="ts">
defineProps<{
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
              新版本 <b class="update-version">v{{ version }}</b> 已发布，是否立即下载并安装？
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
