<template>
  <Transition name="modal-fade">
    <div v-if="show" class="modal-overlay" @mousedown.self="emit('close')">
      <div class="modal-card font-selector-modal-card">
        <div class="modal-header">
          <span class="modal-title">更换界面字体 (全局生效)</span>
          <span class="modal-close" @click="emit('close')">✕</span>
        </div>
        <div class="modal-body font-selector-body">
          <div class="font-selector-toolbar">
            <label class="md3-checkbox-label">
              <input
                type="checkbox"
                :checked="includeSystemFonts"
                class="md3-check-native"
                @change="onToggleSystemFonts"
              />
              <span>读取系统字体</span>
            </label>
            <span v-if="isReadingSystemFonts" class="font-loading-tip">正在读取系统中...</span>
          </div>

          <div class="font-list-scrollable">
            <!-- 推荐默认与精选字体 -->
            <div class="font-group-title">精选与默认界面字体</div>
            <div
              v-for="item in presetFonts"
              :key="item.value"
              :class="['font-item-row', appFontFamily === item.value ? 'active' : '']"
              @click="emit('apply-font', item.value)"
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
                @click="emit('apply-font', `'${f}', sans-serif`)"
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
          <button class="btn-confirm" @click="emit('close')">完成选择</button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
const props = defineProps<{
  show: boolean;
  presetFonts: Array<{ name: string; value: string }>;
  appFontFamily: string;
  systemFontList: string[];
  includeSystemFonts: boolean;
  isReadingSystemFonts: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "update:includeSystemFonts", value: boolean): void;
  (e: "toggle-system-fonts"): void;
  (e: "apply-font", value: string): void;
}>();

function onToggleSystemFonts(event: Event) {
  const checked = (event.target as HTMLInputElement).checked;
  emit("update:includeSystemFonts", checked);
  emit("toggle-system-fonts");
}
</script>
