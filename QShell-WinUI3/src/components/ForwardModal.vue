<template>
  <Transition name="modal-fade">
    <div v-if="modal.show" class="modal-overlay" @mousedown.self="emit('close')">
      <div class="modal-card forward-modal">
        <div class="modal-header">
          <span class="modal-title">端口转发</span>
          <span class="modal-close" @click="emit('close')">✕</span>
        </div>
        <div class="modal-body">
          <div class="forward-type-switch">
            <button
              :class="['forward-type-btn', modal.draft.type === 'local' ? 'active' : '']"
              @click="modal.draft.type = 'local'"
            >本地转发 (-L)</button>
            <button
              :class="['forward-type-btn', modal.draft.type === 'remote' ? 'active' : '']"
              @click="modal.draft.type = 'remote'"
            >远程转发 (-R)</button>
            <button
              :class="['forward-type-btn', modal.draft.type === 'socks' ? 'active' : '']"
              @click="modal.draft.type = 'socks'"
            >SOCKS5 (-D)</button>
          </div>

          <p v-if="modal.draft.type === 'local'" class="confirm-modal-text">
            浏览器访问 <b>127.0.0.1:{{ modal.draft.localPort || "本机端口" }}</b>
            → 实际连接到服务器上的
            <b>{{ modal.draft.remoteHost || "目标地址" }}:{{ modal.draft.remotePort || "目标端口" }}</b>
          </p>
          <p v-else-if="modal.draft.type === 'remote'" class="confirm-modal-text">
            服务器监听 <b>:{{ modal.draft.bindPort || "远端端口" }}</b>
            → 转发到本机的
            <b>{{ modal.draft.targetHost || "本地地址" }}:{{ modal.draft.targetPort || "本地端口" }}</b>
          </p>
          <p v-else class="confirm-modal-text">
            浏览器配置 SOCKS5 代理 <b>127.0.0.1:{{ modal.draft.socksPort || "1080" }}</b>
            → 所有流量经服务器代理转发
          </p>

          <div v-if="modal.draft.type === 'local'" class="forward-draft-row">
            <input v-model.number="modal.draft.localPort" type="number" class="modal-input" placeholder="本机端口（浏览器访问）" style="width: 150px;" />
            <span class="forward-arrow">→</span>
            <input v-model="modal.draft.remoteHost" class="modal-input" placeholder="目标地址（服务器上的）" style="flex: 1;" />
            <input v-model.number="modal.draft.remotePort" type="number" class="modal-input" placeholder="目标端口" style="width: 100px;" />
            <button class="btn-sec-act" @click="emit('add')">添加</button>
          </div>
          <div v-else-if="modal.draft.type === 'remote'" class="forward-draft-row">
            <input v-model.number="modal.draft.bindPort" type="number" class="modal-input" placeholder="远端监听端口" style="width: 130px;" />
            <span class="forward-arrow">←</span>
            <input v-model="modal.draft.targetHost" class="modal-input" placeholder="本地目标地址" style="flex: 1;" />
            <input v-model.number="modal.draft.targetPort" type="number" class="modal-input" placeholder="本地端口" style="width: 100px;" />
            <button class="btn-sec-act" @click="emit('add')">添加</button>
          </div>
          <div v-else class="forward-draft-row">
            <input v-model.number="modal.draft.socksPort" type="number" class="modal-input" placeholder="本地 SOCKS5 端口" style="width: 150px;" />
            <span class="forward-arrow">→</span>
            <span class="forward-draft-hint">经 SSH 通道代理所有 TCP 连接</span>
            <button class="btn-sec-act" @click="emit('add')">添加</button>
          </div>

          <div v-if="modal.rules.length === 0" class="sftp-empty">还没有转发规则</div>
          <div v-for="rule in modal.rules" :key="rule.id" class="forward-rule-block">
            <span :class="['forward-type-tag', rule.type]">{{ typeLabel(rule.type) }}</span>
            <span class="forward-rule-desc">
              <template v-if="rule.type === 'remote'">服务器 :{{ modal.actualPorts[rule.id] || rule.bindPort }} → 本机 {{ rule.targetHost }}:{{ rule.targetPort }}</template>
              <template v-else-if="rule.type === 'socks'">SOCKS5 127.0.0.1:{{ modal.actualPorts[rule.id] || rule.localPort }} → 经服务器代理</template>
              <template v-else>浏览器 127.0.0.1:{{ modal.actualPorts[rule.id] || rule.localPort }} → 服务器 {{ rule.remoteHost }}:{{ rule.remotePort }}</template>
            </span>
            <span :class="['forward-status', modal.running[rule.id] ? 'on' : 'off']">{{ modal.running[rule.id] ? "运行中" : "已停止" }}</span>
            <div class="forward-rule-actions">
              <button
                class="btn-sec-act"
                @click="modal.running[rule.id] ? emit('stop', rule) : emit('start', rule)"
              >{{ modal.running[rule.id] ? "停止" : "启动" }}</button>
              <button class="btn-cancel" @click="emit('remove', rule)">删除</button>
            </div>
            <div v-if="modal.errors[rule.id]" class="forward-error-line">⚠ {{ modal.errors[rule.id] }}</div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="emit('close')">关闭</button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
export interface ForwardRule {
  id: string;
  type: "local" | "remote" | "socks";
  localPort: number;
  remoteHost: string;
  remotePort: number;
  bindPort: number;
  targetHost: string;
  targetPort: number;
}

export interface ForwardDraft {
  type: "local" | "remote" | "socks";
  localPort: number;
  remoteHost: string;
  remotePort: number;
  bindPort: number;
  targetHost: string;
  targetPort: number;
  socksPort: number;
}

export interface ForwardModalState {
  show: boolean;
  hostKey: string;
  rules: ForwardRule[];
  running: Record<string, boolean>;
  actualPorts: Record<string, number>;
  errors: Record<string, string>;
  draft: ForwardDraft;
}

const props = defineProps<{ modal: ForwardModalState }>();

const emit = defineEmits<{
  (e: "add"): void;
  (e: "start", rule: ForwardRule): void;
  (e: "stop", rule: ForwardRule): void;
  (e: "remove", rule: ForwardRule): void;
  (e: "close"): void;
}>();

function typeLabel(type: ForwardRule["type"]) {
  if (type === "remote") return "远程 -R";
  if (type === "socks") return "SOCKS5 -D";
  return "本地 -L";
}
</script>
