<template>
  <Transition name="modal-fade">
    <div v-if="show" class="modal-overlay" @mousedown.self="emitClose">
      <div class="modal-card trace-modal">
        <div class="modal-header">
          <span class="modal-title">路由追踪 · {{ title }}</span>
          <span class="modal-close" @click="emitClose">✕</span>
        </div>
        <div class="trace-body">
          <div class="trace-target-row">
            <span class="trace-target-label">目标</span>
            <span class="trace-target-ip">{{ host }}</span>
          </div>
          <div v-if="error" class="trace-error">{{ error }}</div>
          <template v-else>
            <div v-if="hops.length > 0" class="trace-list">
              <div
                v-for="hop in hops"
                :key="hop.hop"
                class="trace-hop"
                :class="{ 'is-timeout': hop.timeout && !hop.ip }"
              >
                <span class="trace-hop-index">{{ hop.hop }}</span>
                <div class="trace-hop-main">
                  <div class="trace-hop-ip-row">
                    <span class="trace-hop-ip" :title="hop.ip || '请求超时'">{{ hop.ip || "请求超时" }}</span>
                    <span v-if="hop.location" class="trace-hop-location">{{ hop.location }}</span>
                  </div>
                  <div class="trace-bar-track">
                    <div class="trace-bar" :class="barClass(hop)" :style="{ width: barWidth(hop) + '%' }"></div>
                  </div>
                </div>
                <span class="trace-hop-rtt">{{ rttText(hop) }}</span>
              </div>
            </div>
            <div v-if="loading" class="trace-loading">
              <span class="trace-spinner"></span>
              <span>正在追踪路由...（已发现 {{ hops.length }} 跳）</span>
            </div>
            <div v-else-if="hops.length === 0" class="trace-loading">未发现任何路由节点</div>
          </template>
        </div>
        <div class="modal-footer">
          <button class="btn-confirm" @click="emitClose">关闭</button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
export interface TraceHop {
  hop: number;
  ip: string;
  rtt_ms: number[];
  timeout: boolean;
  location: string | null;
  isp: string | null;
}

const props = defineProps<{
  show: boolean;
  title: string;
  host: string;
  loading: boolean;
  error: string;
  hops: TraceHop[];
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

function emitClose() {
  emit("close");
}

function traceAvg(hop: TraceHop): number | null {
  if (!hop.rtt_ms || hop.rtt_ms.length === 0) return null;
  return hop.rtt_ms.reduce((a, b) => a + b, 0) / hop.rtt_ms.length;
}

function traceMaxRtt(): number {
  let max = 0;
  for (const hop of props.hops) {
    const avg = traceAvg(hop);
    if (avg !== null && avg > max) max = avg;
  }
  return Math.max(max, 20);
}

function barWidth(hop: TraceHop): number {
  const avg = traceAvg(hop);
  if (avg === null) return 0;
  return Math.min(100, Math.max(8, (avg / traceMaxRtt()) * 100));
}

function barClass(hop: TraceHop): string {
  const avg = traceAvg(hop);
  if (avg === null) return "is-timeout";
  if (avg < 50) return "is-fast";
  if (avg < 150) return "is-mid";
  return "is-slow";
}

function rttText(hop: TraceHop): string {
  if (!hop.rtt_ms || hop.rtt_ms.length === 0) return "*";
  return hop.rtt_ms.map((v) => `${v.toFixed(v < 10 ? 1 : 0)} ms`).join(" · ");
}
</script>
