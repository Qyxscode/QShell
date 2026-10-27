<script setup lang="ts">
/**
 * 拖拽上传落点提示层。
 * 只做展示：拖拽期间覆盖在目标窗格上，告诉用户"松手会传到哪个会话的哪个目录"。
 * 不接收指针事件，否则原生拖放的 drop 会被这一层挡住。
 */
const props = defineProps<{
  visible: boolean;
  /** 与目标窗格一致的 CSS grid-area */
  area: string;
  sessionName: string;
  dir: string;
  count: number;
  /** 目标不可用时为 true，只提示不落盘 */
  blocked: boolean;
  /** empty=空窗格；unsupported=会话不支持文件传输 */
  reason: "empty" | "unsupported" | "";
}>();

const title = () => (props.blocked ? (props.reason === "empty" ? "这一屏还没有会话" : "无法上传到此屏") : "松开上传到此屏");
const hint = () =>
  props.reason === "empty"
    ? "请先为这一屏选择会话，再拖入文件"
    : props.reason === "unsupported"
      ? "会话未连接，或当前协议不支持文件传输"
      : "";
</script>

<template>
  <div
    v-if="visible"
    class="drop-upload-overlay"
    :class="{ 'is-blocked': blocked }"
    :style="{ gridArea: area }"
  >
    <div class="drop-upload-card">
      <div class="duo-head">
        <span class="duo-icon">{{ blocked ? "!" : "↓" }}</span>
        <span class="duo-title">{{ title() }}</span>
      </div>
      <div v-if="!blocked || reason === 'unsupported'" class="duo-session">
        <i class="duo-dot"></i>
        <span class="duo-name">{{ sessionName }}</span>
      </div>
      <div v-if="!blocked && dir" class="duo-dir" :title="dir">{{ dir }}</div>
      <div v-if="!blocked && count > 1" class="duo-count">{{ count }} 个项目 · 依次上传</div>
      <div v-if="hint()" class="duo-hint">{{ hint() }}</div>
    </div>
  </div>
</template>
