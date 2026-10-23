<script setup lang="ts">
import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import CustomSelect from "./CustomSelect.vue";

const props = defineProps<{
  modelValue: boolean;
  defaultComment?: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: boolean): void;
  (e: "generated", payload: { privatePath: string; passphrase: string }): void;
}>();

const keyKindOptions = [
  { label: "ED25519（推荐）", value: "ed25519" },
  { label: "RSA 4096", value: "rsa4096" },
  { label: "RSA 2048", value: "rsa2048" },
  { label: "ECDSA P-256", value: "ecdsa" },
];

const kind = ref("ed25519");
const directory = ref("");
const fileName = ref("id_ed25519");
const passphrase = ref("");
const comment = ref("");
const generating = ref(false);
const error = ref("");
const result = ref<null | {
  private_path: string;
  public_path: string;
  public_key: string;
  fingerprint: string;
  algorithm: string;
}>(null);

watch(
  () => props.modelValue,
  (show) => {
    if (!show) return;
    kind.value = "ed25519";
    directory.value = "";
    fileName.value = "id_ed25519";
    passphrase.value = "";
    comment.value = props.defaultComment || "";
    generating.value = false;
    error.value = "";
    result.value = null;
  }
);

function close() {
  emit("update:modelValue", false);
}

function onKindChange(value: string) {
  kind.value = value;
  const defaults: Record<string, string> = {
    ed25519: "id_ed25519",
    rsa4096: "id_rsa4096",
    rsa2048: "id_rsa",
    ecdsa: "id_ecdsa",
  };
  fileName.value = defaults[value] || "id_qshell";
}

async function generate() {
  generating.value = true;
  error.value = "";
  try {
    const info = await invoke<any>("generate_ssh_key", {
      kind: kind.value,
      directory: directory.value.trim() || null,
      fileName: fileName.value,
      passphrase: passphrase.value || null,
      comment: comment.value || null,
    });
    result.value = info;
    emit("generated", {
      privatePath: info.private_path,
      passphrase: passphrase.value,
    });
  } catch (e: any) {
    error.value = String(e?.message || e);
  } finally {
    generating.value = false;
  }
}

async function copyPublicKey() {
  if (result.value?.public_key) {
    await navigator.clipboard.writeText(result.value.public_key);
  }
}
</script>

<template>
  <Transition name="modal-fade">
    <div v-if="modelValue" class="modal-overlay" @mousedown.self="close">
      <div class="modal-card modal-small">
        <div class="modal-header">
          <span class="modal-title">生成 SSH 密钥</span>
          <span class="modal-close" @click="close">✕</span>
        </div>
        <div class="modal-body">
          <div class="form-row">
            <label>密钥类型</label>
            <CustomSelect
              :modelValue="kind"
              :options="keyKindOptions"
              customClass="w-full"
              @change="onKindChange"
            />
          </div>
          <div class="form-row">
            <label>保存目录</label>
            <input v-model="directory" class="modal-input" placeholder="留空 = 用户目录下的 .ssh 文件夹" />
          </div>
          <div class="form-row">
            <label>文件名</label>
            <input v-model="fileName" class="modal-input" />
          </div>
          <div class="form-row">
            <label>口令（可选）</label>
            <input v-model="passphrase" type="password" class="modal-input" placeholder="留空表示无口令" />
          </div>
          <div class="form-row">
            <label>注释（可选）</label>
            <input v-model="comment" class="modal-input" />
          </div>
          <div v-if="error" class="infobar-warning" style="margin-top: 8px;">
            <span class="infobar-icon">⚠️</span>
            <span class="infobar-text">{{ error }}</span>
          </div>
          <div v-if="result" class="keygen-result">
            <div class="keygen-line">私钥：{{ result.private_path }}</div>
            <div class="keygen-line">公钥：{{ result.public_path }}</div>
            <div class="keygen-line">指纹：{{ result.fingerprint }}</div>
            <div class="keygen-pubkey">{{ result.public_key }}</div>
          </div>
        </div>
        <div class="modal-footer">
          <button class="btn-cancel" @click="close">关闭</button>
          <button v-if="result" class="btn-sec-act" @click="copyPublicKey">复制公钥</button>
          <button class="btn-confirm" :disabled="generating" @click="generate">
            {{ generating ? "生成中..." : "生成" }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>
