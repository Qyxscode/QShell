<template>
  <div class="custom-datetime-picker" :class="[customClass, { disabled }]" ref="pickerRef">
    <div 
      class="picker-input-field modal-input" 
      :class="{ 'is-focused': isOpen, 'is-disabled': disabled }"
      @click="togglePicker"
    >
      <span class="display-val" :class="{ 'is-placeholder': !displayValue }">
        {{ displayValue || (showSeconds ? 'YYYY/MM/DD --:--:--' : 'YYYY/MM/DD --:--') }}
      </span>
      <button 
        type="button" 
        class="calendar-trigger-btn" 
        :disabled="disabled"
        tabindex="-1"
        title="选择到期时间"
      >
        <svg class="calendar-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor">
          <rect x="3" y="4" width="18" height="18" rx="3" stroke-width="2" />
          <line x1="16" y1="2" x2="16" y2="6" stroke-width="2" stroke-linecap="round" />
          <line x1="8" y1="2" x2="8" y2="6" stroke-width="2" stroke-linecap="round" />
          <line x1="3" y1="10" x2="21" y2="10" stroke-width="2" />
          <rect x="7" y="13" width="2.5" height="2.5" rx="0.5" fill="currentColor" stroke="none" />
          <rect x="11.5" y="13" width="2.5" height="2.5" rx="0.5" fill="currentColor" stroke="none" />
          <rect x="16" y="13" width="2.5" height="2.5" rx="0.5" fill="currentColor" stroke="none" />
          <rect x="7" y="17" width="2.5" height="2.5" rx="0.5" fill="currentColor" stroke="none" />
          <rect x="11.5" y="17" width="2.5" height="2.5" rx="0.5" fill="currentColor" stroke="none" />
        </svg>
      </button>
    </div>

    <!-- 弹出的自定义高级日历面板 -->
    <teleport to="body">
      <transition name="popover-fade">
        <div
          v-if="isOpen"
          ref="panelRef"
          class="picker-dropdown-panel"
          :class="{ 'has-seconds': showSeconds }"
          :style="panelStyle"
          @click.stop
        >
          <div class="panel-main-content">
            <!-- 左侧：日历区 -->
            <div class="calendar-pane">
            <div class="cal-nav-bar">
              <span class="current-ym-title">{{ currentYear }}年{{ currentMonth + 1 }}月</span>
              <div class="nav-arrow-group">
                <button type="button" class="nav-btn" @click.stop="prevMonth" title="上个月">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                    <polyline points="15 18 9 12 15 6" />
                  </svg>
                </button>
                <button type="button" class="nav-btn" @click.stop="nextMonth" title="下个月">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                    <polyline points="9 18 15 12 9 6" />
                  </svg>
                </button>
              </div>
            </div>

            <div class="cal-weekdays">
              <span v-for="w in ['一', '二', '三', '四', '五', '六', '日']" :key="w">{{ w }}</span>
            </div>

            <div class="cal-days-grid">
              <div 
                v-for="d in calendarDays" 
                :key="d.key"
                class="day-cell"
                :class="{
                  'other-month': !d.isCurrentMonth,
                  'is-today': d.isToday,
                  'is-selected': isDaySelected(d)
                }"
                @click="selectDay(d)"
              >
                {{ d.day }}
              </div>
            </div>
          </div>

          <!-- 右侧：时间滚轮选择器 (时 / 分 / 秒) -->
          <div class="time-pane" :class="{ 'has-seconds': showSeconds }">
            <div class="time-column hour-col">
              <div class="time-header">时</div>
              <div class="time-scroll-list" ref="hourListRef">
                <div 
                  v-for="h in 24" 
                  :key="h - 1"
                  class="time-cell"
                  :class="{ active: selectedHour === (h - 1) }"
                  @click="selectedHour = (h - 1)"
                >
                  {{ String(h - 1).padStart(2, '0') }}
                </div>
              </div>
            </div>

            <div class="time-column min-col">
              <div class="time-header">分</div>
              <div class="time-scroll-list" ref="minListRef">
                <div 
                  v-for="m in 60" 
                  :key="m - 1"
                  class="time-cell"
                  :class="{ active: selectedMinute === (m - 1) }"
                  @click="selectedMinute = (m - 1)"
                >
                  {{ String(m - 1).padStart(2, '0') }}
                </div>
              </div>
            </div>

            <div v-if="showSeconds" class="time-column sec-col">
              <div class="time-header">秒</div>
              <div class="time-scroll-list" ref="secondListRef">
                <div 
                  v-for="s in 60" 
                  :key="s - 1"
                  class="time-cell"
                  :class="{ active: selectedSecond === (s - 1) }"
                  @click="selectedSecond = (s - 1)"
                >
                  {{ String(s - 1).padStart(2, '0') }}
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- 底部快捷操作条 -->
        <div class="picker-footer">
          <button type="button" class="footer-btn clear" @click="clearValue">清除</button>
          <div class="footer-right">
            <button type="button" class="footer-btn today" @click="pickToday">今天</button>
            <button type="button" class="footer-btn confirm" @click="confirmSelection">确定</button>
          </div>
        </div>
        </div>
      </transition>
    </teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";

const props = withDefaults(defineProps<{
  modelValue?: string;
  disabled?: boolean;
  customClass?: string;
  showSeconds?: boolean;
}>(), {
  modelValue: "",
  disabled: false,
  customClass: "",
  showSeconds: false,
});

const emit = defineEmits<{
  (e: "update:modelValue", val: string): void;
  (e: "change", val: string): void;
}>();

const isOpen = ref(false);
const pickerRef = ref<HTMLElement | null>(null);
const panelRef = ref<HTMLElement | null>(null);
const panelStyle = ref<Record<string, string>>({
  position: "fixed",
  left: "-9999px",
  top: "-9999px",
  zIndex: "10050",
});

const currentYear = ref(new Date().getFullYear());
const currentMonth = ref(new Date().getMonth()); // 0-11
const selectedDate = ref<Date | null>(null);
const selectedHour = ref(0);
const selectedMinute = ref(0);
const selectedSecond = ref(0);

const hourListRef = ref<HTMLElement | null>(null);
const minListRef = ref<HTMLElement | null>(null);
const secondListRef = ref<HTMLElement | null>(null);

// 解析 modelValue (支持 YYYY/MM/DD HH:mm[:ss], YYYY-MM-DDTHH:mm[:ss], YYYY-MM-DD HH:mm[:ss])
function parseModelValue(val: string) {
  if (!val || val === "无限") {
    selectedDate.value = null;
    return;
  }
  const clean = val.replace("T", " ").replace(/-/g, "/").trim();
  const parts = clean.split(" ");
  if (parts.length >= 1) {
    const ymd = parts[0].split("/");
    if (ymd.length === 3) {
      const y = parseInt(ymd[0], 10);
      const m = parseInt(ymd[1], 10) - 1;
      const d = parseInt(ymd[2], 10);
      currentYear.value = y;
      currentMonth.value = m;
      selectedDate.value = new Date(y, m, d);
    }
  }
  if (parts.length >= 2) {
    const hms = parts[1].split(":");
    if (hms.length >= 2) {
      selectedHour.value = parseInt(hms[0], 10) || 0;
      selectedMinute.value = parseInt(hms[1], 10) || 0;
      selectedSecond.value = hms.length >= 3 ? (parseInt(hms[2], 10) || 0) : 0;
    }
  }
}

watch(() => props.modelValue, (newVal) => {
  if (newVal) {
    parseModelValue(newVal);
  } else {
    selectedDate.value = null;
  }
}, { immediate: true });

// 格式化输出为 YYYY/MM/DD HH:mm[:ss]
const displayValue = computed(() => {
  if (!selectedDate.value) return "";
  const y = selectedDate.value.getFullYear();
  const m = String(selectedDate.value.getMonth() + 1).padStart(2, "0");
  const d = String(selectedDate.value.getDate()).padStart(2, "0");
  const hh = String(selectedHour.value).padStart(2, "0");
  const mm = String(selectedMinute.value).padStart(2, "0");
  const ss = String(selectedSecond.value).padStart(2, "0");
  return props.showSeconds
    ? `${y}/${m}/${d} ${hh}:${mm}:${ss}`
    : `${y}/${m}/${d} ${hh}:${mm}`;
});

interface CalDay {
  year: number;
  month: number;
  day: number;
  isCurrentMonth: boolean;
  isToday: boolean;
  key: string;
}

const calendarDays = computed<CalDay[]>(() => {
  const days: CalDay[] = [];
  const y = currentYear.value;
  const m = currentMonth.value;

  const today = new Date();
  const todayY = today.getFullYear();
  const todayM = today.getMonth();
  const todayD = today.getDate();

  // 本月第一天是周几 (0:周日, 1:周一 ... 6:周六)
  const firstDay = new Date(y, m, 1).getDay();
  // 中国惯用 周一为 0，周日为 6
  const startOffset = (firstDay + 6) % 7;

  // 上月天数
  const prevMonthDaysCount = new Date(y, m, 0).getDate();
  for (let i = startOffset - 1; i >= 0; i--) {
    const d = prevMonthDaysCount - i;
    days.push({
      year: m === 0 ? y - 1 : y,
      month: m === 0 ? 11 : m - 1,
      day: d,
      isCurrentMonth: false,
      isToday: false,
      key: `prev-${d}`
    });
  }

  // 本月天数
  const curMonthDaysCount = new Date(y, m + 1, 0).getDate();
  for (let d = 1; d <= curMonthDaysCount; d++) {
    const isToday = (todayY === y && todayM === m && todayD === d);
    days.push({
      year: y,
      month: m,
      day: d,
      isCurrentMonth: true,
      isToday,
      key: `cur-${d}`
    });
  }

  // 下月补齐至 42 格 (6周)
  const remaining = 42 - days.length;
  for (let d = 1; d <= remaining; d++) {
    days.push({
      year: m === 11 ? y + 1 : y,
      month: m === 11 ? 0 : m + 1,
      day: d,
      isCurrentMonth: false,
      isToday: false,
      key: `next-${d}`
    });
  }

  return days;
});

function isDaySelected(d: CalDay) {
  if (!selectedDate.value) return false;
  return (
    selectedDate.value.getFullYear() === d.year &&
    selectedDate.value.getMonth() === d.month &&
    selectedDate.value.getDate() === d.day
  );
}

function prevMonth() {
  if (currentMonth.value === 0) {
    currentMonth.value = 11;
    currentYear.value--;
  } else {
    currentMonth.value--;
  }
}

function nextMonth() {
  if (currentMonth.value === 11) {
    currentMonth.value = 0;
    currentYear.value++;
  } else {
    currentMonth.value++;
  }
}

function selectDay(d: CalDay) {
  selectedDate.value = new Date(d.year, d.month, d.day);
  currentYear.value = d.year;
  currentMonth.value = d.month;
}

function pickToday() {
  const now = new Date();
  currentYear.value = now.getFullYear();
  currentMonth.value = now.getMonth();
  selectedDate.value = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  selectedHour.value = now.getHours();
  selectedMinute.value = now.getMinutes();
  selectedSecond.value = now.getSeconds();
  scrollTimeToActive();
}

function clearValue() {
  selectedDate.value = null;
  emit("update:modelValue", "");
  emit("change", "");
  isOpen.value = false;
}

function confirmSelection() {
  if (!selectedDate.value) {
    pickToday();
  }
  emit("update:modelValue", displayValue.value);
  emit("change", displayValue.value);
  isOpen.value = false;
}

function togglePicker() {
  if (props.disabled) return;
  isOpen.value = !isOpen.value;
  if (isOpen.value) {
    if (!selectedDate.value && props.modelValue) {
      parseModelValue(props.modelValue);
    }
    nextTick(() => {
      updatePanelPosition();
      scrollTimeToActive();
      requestAnimationFrame(updatePanelPosition);
    });
  }
}

function updatePanelPosition() {
  if (!isOpen.value || !pickerRef.value) return;
  const anchor = pickerRef.value.getBoundingClientRect();
  const width = panelRef.value?.offsetWidth || (props.showSeconds ? 368 : 320);
  const height = panelRef.value?.offsetHeight || 330;
  const margin = 8;

  let left = anchor.left;
  if (left + width > window.innerWidth - margin) {
    left = window.innerWidth - width - margin;
  }
  left = Math.max(margin, left);

  let top = anchor.bottom + 6;
  if (top + height > window.innerHeight - margin) {
    top = anchor.top - height - 6;
  }
  top = Math.max(margin, top);

  panelStyle.value = {
    position: "fixed",
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
    zIndex: "10050",
  };
}

function scrollTimeToActive() {
  if (hourListRef.value) {
    const activeEl = hourListRef.value.querySelector(".time-cell.active") as HTMLElement;
    if (activeEl) {
      hourListRef.value.scrollTop = activeEl.offsetTop - 60;
    }
  }
  if (minListRef.value) {
    const activeEl = minListRef.value.querySelector(".time-cell.active") as HTMLElement;
    if (activeEl) {
      minListRef.value.scrollTop = activeEl.offsetTop - 60;
    }
  }
  if (secondListRef.value) {
    const activeEl = secondListRef.value.querySelector(".time-cell.active") as HTMLElement;
    if (activeEl) {
      secondListRef.value.scrollTop = activeEl.offsetTop - 60;
    }
  }
}

function handleOutsideClick(e: MouseEvent) {
  const target = e.target as Node;
  if (pickerRef.value?.contains(target)) return;
  if (panelRef.value?.contains(target)) return;
  isOpen.value = false;
}

onMounted(() => {
  document.addEventListener("mousedown", handleOutsideClick);
  window.addEventListener("resize", updatePanelPosition);
  window.addEventListener("scroll", updatePanelPosition, true);
});

onUnmounted(() => {
  document.removeEventListener("mousedown", handleOutsideClick);
  window.removeEventListener("resize", updatePanelPosition);
  window.removeEventListener("scroll", updatePanelPosition, true);
});
</script>

<style scoped>
.custom-datetime-picker {
  position: relative;
  width: 100%;
}

.picker-input-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  cursor: pointer;
  user-select: none;
  padding: 0 4px 0 10px !important;
  transition: all 0.2s ease;
}

.picker-input-field.is-disabled {
  opacity: 0.55;
  cursor: not-allowed;
  pointer-events: none;
}

.picker-input-field.is-focused {
  border-color: var(--color-primary) !important;
  box-shadow: 0 0 0 2px rgba(120, 217, 236, 0.18) !important;
}

.display-val {
  font-size: 12px;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, monospace, sans-serif;
  color: var(--text-main);
  letter-spacing: 0.2px;
}

.display-val.is-placeholder {
  color: var(--text-sub);
  opacity: 0.65;
  font-weight: 500;
}

/* 规范化高级日历图标按钮 (24x24px，日夜模式自适应) */
.calendar-trigger-btn {
  width: 24px;
  height: 24px;
  min-width: 24px;
  min-height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-capsule);
  border: 1px solid var(--border-capsule);
  border-radius: 5px;
  color: var(--color-primary);
  cursor: pointer;
  padding: 0;
  transition: all 0.18s ease;
}

.calendar-trigger-btn:hover {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: var(--color-on-primary);
}

.calendar-icon {
  width: 14px;
  height: 14px;
}

/* 弹出的下拉日历主面板 (MD3 / 日间自适应) */
.picker-dropdown-panel {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 1050;
  width: 320px;
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: var(--radius-card);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.35);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  user-select: none;
}

.picker-dropdown-panel.has-seconds {
  width: 368px;
}

[data-theme="light"] .picker-dropdown-panel {
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.14);
}

.panel-main-content {
  display: flex;
  height: 250px;
}

/* 左侧日历 */
.calendar-pane {
  flex: 1;
  padding: 10px 12px;
  border-right: 1px solid var(--border-card);
  display: flex;
  flex-direction: column;
}

.cal-nav-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.current-ym-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-title);
}

.nav-arrow-group {
  display: flex;
  gap: 4px;
}

.nav-btn {
  width: 22px;
  height: 22px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid transparent;
  border-radius: 4px;
  color: var(--text-sub);
  cursor: pointer;
  padding: 0;
  transition: all 0.15s ease;
}

.nav-btn:hover {
  background: var(--bg-capsule-hover);
  color: var(--text-title);
  border-color: var(--border-capsule);
}

.cal-weekdays {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  text-align: center;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-sub);
  margin-bottom: 6px;
}

.cal-days-grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  grid-template-rows: repeat(6, 1fr);
  gap: 2px;
  flex: 1;
}

.day-cell {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 500;
  border-radius: 4px;
  cursor: pointer;
  color: var(--text-main);
  transition: all 0.15s ease;
}

.day-cell:hover:not(.is-selected) {
  background: var(--bg-capsule-hover);
  color: var(--text-title);
}

.day-cell.other-month {
  color: var(--text-sub);
  opacity: 0.35;
}

.day-cell.is-today:not(.is-selected) {
  border: 1px solid var(--color-primary);
  color: var(--color-primary);
}

.day-cell.is-selected {
  background: var(--color-primary);
  color: var(--color-on-primary);
  font-weight: 700;
}

/* 右侧时间列 */
.time-pane {
  width: 96px;
  display: flex;
  background: var(--bg-sub-card);
}

.time-pane.has-seconds {
  width: 144px;
}

.time-column {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.time-column:first-child {
  border-right: 1px solid var(--border-card);
}

.time-header {
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-sub);
  border-bottom: 1px solid var(--border-card);
}

.time-scroll-list {
  flex: 1;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--border-input) transparent;
}

.time-scroll-list:deep(*)::-webkit-scrollbar,
.time-scroll-list::-webkit-scrollbar {
  width: 5px !important;
  background: transparent !important;
}

.time-scroll-list:deep(*)::-webkit-scrollbar-track,
.time-scroll-list::-webkit-scrollbar-track {
  background: transparent !important;
}

.time-scroll-list:deep(*)::-webkit-scrollbar-thumb,
.time-scroll-list::-webkit-scrollbar-thumb {
  background-color: var(--border-input) !important;
  border-radius: 4px !important;
}

.time-scroll-list:deep(*)::-webkit-scrollbar-thumb:hover,
.time-scroll-list::-webkit-scrollbar-thumb:hover {
  background-color: var(--color-primary) !important;
}

.time-cell {
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 500;
  font-family: Consolas, monospace;
  color: var(--text-main);
  cursor: pointer;
  transition: all 0.15s ease;
}

.time-cell:hover:not(.active) {
  background: var(--bg-capsule-hover);
}

.time-cell.active {
  background: var(--color-primary);
  color: var(--color-on-primary);
  font-weight: 700;
}

/* 底部按钮栏 */
.picker-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-top: 1px solid var(--border-card);
  background: var(--bg-sub-card);
}

.footer-btn {
  height: 24px;
  padding: 0 10px;
  font-size: 11px;
  font-weight: 600;
  border-radius: var(--radius-btn);
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.15s ease;
}

.footer-btn.clear {
  background: transparent;
  color: var(--text-sub);
}

.footer-btn.clear:hover {
  background: var(--bg-capsule-hover);
  color: var(--text-title);
}

.footer-right {
  display: flex;
  gap: 6px;
}

.footer-btn.today {
  background: var(--bg-capsule);
  border-color: var(--border-capsule);
  color: var(--text-main);
}

.footer-btn.today:hover {
  background: var(--bg-capsule-hover);
}

.footer-btn.confirm {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: var(--color-on-primary);
}

.footer-btn.confirm:hover {
  background: var(--color-primary-hover);
}

/* 展开动效 */
.popover-fade-enter-active,
.popover-fade-leave-active {
  transition: opacity 0.18s cubic-bezier(0.4, 0, 0.2, 1), transform 0.18s cubic-bezier(0.4, 0, 0.2, 1);
}

.popover-fade-enter-from,
.popover-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.98);
}
</style>
