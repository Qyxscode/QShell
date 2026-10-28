<template>
  <!-- 1. 经典横向文件夹矢量图标 (比例拉长，辨识度高) -->
  <svg v-if="name === 'folder'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M2 6.5C2 5.4 2.9 4.5 4 4.5H9C9.7 4.5 10.3 4.9 10.6 5.5L11.8 7.5H20C21.1 7.5 22 8.4 22 9.5V17.5C22 18.6 21.1 19.5 20 19.5H4C2.9 19.5 2 18.6 2 17.5V6.5Z" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" fill="currentColor" fill-opacity="0.14"/>
    <path d="M2 9.5H22" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" opacity="0.35"/>
  </svg>

  <!-- 1.1 独立展开/折叠三角指示图标 -->
  <svg v-else-if="name === 'chevron'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path v-if="expanded" d="M6 9L12 15L18 9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
    <path v-else d="M9 6L15 12L9 18" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 2. 主机设备矢量图标 (精致屏幕 + 提示符 + 底座，无多余残影) -->
  <svg v-else-if="name === 'host'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <rect x="2" y="3.5" width="20" height="13" rx="2.5" stroke="currentColor" stroke-width="1.6" fill="currentColor" fill-opacity="0.08" />
    <path d="M6 7.5L9 9.5L6 11.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
    <path d="M11 11.5H16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
    <path d="M12 16.5V19.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
    <path d="M8 19.5H16" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
  </svg>

  <!-- 3. 文件/文档矢量图标 (纸张轮廓 + 段落线条) -->
  <svg v-else-if="name === 'file'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M6 3H14L19 8V20C19 20.6 18.6 21 18 21H6C5.4 21 5 20.6 5 20V4C5 3.4 5.4 3 6 3Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" fill="currentColor" fill-opacity="0.08"/>
    <path d="M14 3V8H19" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/>
    <path d="M9 12H15" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
    <path d="M9 15H15" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
    <path d="M9 18H12" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
  </svg>

  <!-- 4. 偏好设置/机械六齿工业齿轮图标 (外圈六齿原位自转，中心圆孔稳固不偏，2.1px 饱满线宽) -->
  <svg v-else-if="name === 'settings'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon icon-settings">
    <g class="gear-outer-teeth">
      <path 
        d="M 17.00 10.56 L 19.29 10.75 L 19.29 13.25 L 17.00 13.44 L 15.74 15.61 L 16.73 17.69 L 14.56 18.94 L 13.25 17.05 L 10.75 17.05 L 9.44 18.94 L 7.27 17.69 L 8.26 15.61 L 7.00 13.44 L 4.71 13.25 L 4.71 10.75 L 7.00 10.56 L 8.26 8.39 L 7.27 6.31 L 9.44 5.06 L 10.75 6.95 L 13.25 6.95 L 14.56 5.06 L 16.73 6.31 L 15.74 8.39 Z" 
        stroke="currentColor" 
        stroke-width="2.1" 
        stroke-linejoin="round"
        stroke-linecap="round"
      />
    </g>
    <circle cx="12" cy="12" r="2.5" stroke="currentColor" stroke-width="2.1"/>
  </svg>

  <!-- 5. 云同步/备份矢量图标 -->
  <svg v-else-if="name === 'cloud'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M18 10H17.3C16.8 6.6 13.8 4 10.2 4.2C7.3 4.4 4.8 6.6 4.3 9.5C2.1 10.4 0.7 12.6 1 15C1.3 17.5 3.5 19.4 6 19.4H18C20.8 19.4 23 17.2 23 14.4C23 11.8 20.8 9.8 18 10Z" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" fill="currentColor" fill-opacity="0.12"/>
    <path d="M12 12V17M12 12L9.5 14.5M12 12L14.5 14.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 6. 警告/Alert矢量图标 -->
  <svg v-else-if="name === 'alert'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M12 3L2 20H22L12 3Z" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" fill="currentColor" fill-opacity="0.15"/>
    <path d="M12 9V14" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>
    <circle cx="12" cy="17" r="1" fill="currentColor"/>
  </svg>

  <!-- 24. 全屏图标（四角箭头） -->
  <svg v-else-if="name === 'fullscreen'" :width="size || 20" :height="size || 20" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M4 9V4H9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M20 9V4H15" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M4 15V20H9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M20 15V20H15" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 7. 闪电/极速终端图标 (整体缩放至与铃铛/月亮同视觉尺寸，2.1px 线宽) -->
  <svg v-else-if="name === 'bolt'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M12.8 4.2L5 13.6H12L11.2 19.8L19 10.4H12L12.8 4.2Z" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" fill="currentColor" fill-opacity="0.2"/>
  </svg>

  <!-- 8. 月亮深色模式图标 (2.1px 饱满线宽 + 居中圆润饱满月牙) -->
  <svg v-else-if="name === 'moon'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon icon-moon">
    <path d="M19.5 13C18.4 17.2 14.3 19.8 10 18.5C6.7 17.5 4.5 14.5 4.5 11C4.5 7.2 7.2 4.2 10.8 3.5C10 5.6 10.4 8 11.8 9.8C13.4 11.7 15.8 12.6 18.2 12.2C18.7 12.4 19.1 12.7 19.5 13Z" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 9. 铃铛告警/通知图标 (2.1px 饱满线宽 + 居中对称) -->
  <svg v-else-if="name === 'bell'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon icon-bell">
    <path d="M10 5C10 3.9 10.9 3 12 3C13.1 3 14 3.9 14 5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <path d="M7 15.5C7 10.5 8.2 7 12 7C15.8 7 17 10.5 17 15.5L18.8 17.2H5.2L7 15.5Z" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
    <line x1="10.2" y1="19.5" x2="13.8" y2="19.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
  </svg>

  <!-- 10. 传输双向箭头图标 (2.1px 饱满线宽 + 居中对称) -->
  <svg v-else-if="name === 'arrows'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon icon-transfer">
    <!-- 上升箭头 (左轨) -->
    <g class="arrow-up-group">
      <line x1="9" y1="5.5" x2="9" y2="18.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
      <path d="M6 8.5L9 5.5L12 8.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
    </g>
    <!-- 下降箭头 (右轨) -->
    <g class="arrow-down-group">
      <line x1="15" y1="5.5" x2="15" y2="18.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
      <path d="M12 15.5L15 18.5L18 15.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
    </g>
  </svg>

  <!-- 10.1 广播/群发图标 (双向右箭头：箭头头部放大到 4.3×7.2，13px 下也看得清方向) -->
  <svg v-else-if="name === 'broadcast'" :width="size || 14" :height="size || 14" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M6 8.2h10.2" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <path d="m13.1 4.6 4.3 3.6-4.3 3.6" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M6 15.8h10.2" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <path d="m13.1 12.2 4.3 3.6-4.3 3.6" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 10.2 重置/清零 (顺时针回环箭头) -->
  <svg v-else-if="name === 'refresh'" :width="size || 14" :height="size || 14" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M19.5 4.6V9H15.1" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 11. 侧边栏折叠/展开图标 (2.1px 饱满线宽 + 居中外框) -->
  <svg v-else-if="name === 'panel'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon icon-collapse">
    <rect x="5" y="5" width="14" height="14" rx="2.5" stroke="currentColor" stroke-width="2.1"/>
    <line x1="13" y1="5" x2="13" y2="19" stroke="currentColor" stroke-width="2.1"/>
    <path class="collapse-arrow" d="M15 10.5L17 12L15 13.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 12. 备忘录条目图标 (纸张+横线) -->
  <svg v-else-if="name === 'doc'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <rect x="4" y="3" width="16" height="18" rx="2" stroke="currentColor" stroke-width="1.5"/>
    <path d="M8 8H16M8 12H16M8 16H13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
  </svg>

  <!-- 13. 太阳亮色模式图标 (2.1px 饱满线宽 + 均衡比例) -->
  <svg v-else-if="name === 'sun'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon icon-sun">
    <circle cx="12" cy="12" r="3.6" stroke="currentColor" stroke-width="2.1"/>
    <line x1="12" y1="4.8" x2="12" y2="2.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="12" y1="19.2" x2="12" y2="21.5" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="4.8" y1="12" x2="2.5" y2="12" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="19.2" y1="12" x2="21.5" y2="12" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="6.9" y1="6.9" x2="5.3" y2="5.3" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="17.1" y1="17.1" x2="18.7" y2="18.7" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="6.9" y1="17.1" x2="5.3" y2="18.7" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
    <line x1="17.1" y1="6.9" x2="18.7" y2="5.3" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
  </svg>

  <!-- 14. 搜索放大镜图标 -->
  <svg v-else-if="name === 'search'" :width="size" :height="size" viewBox="0 0 24 24" fill="none" class="v-icon">
    <circle cx="11" cy="11" r="7" stroke="currentColor" stroke-width="1.8"/>
    <path d="M16.5 16.5L21 21" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
  </svg>

  <!-- 15. Windows 标题栏: 最小化图标 (与最大化/关闭统一 1.6px 饱满圆角线宽) -->
  <svg v-else-if="name === 'win-min'" :width="size || 11" :height="size || 11" viewBox="0 0 12 12" fill="none" class="v-icon">
    <line x1="1" y1="6" x2="11" y2="6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>
  </svg>

  <!-- 16. Windows 标题栏: 最大化/还原图标 (统一 1.6px 线宽 + 1.8px 微圆角，与图2参考完全一致) -->
  <svg v-else-if="name === 'win-max'" :width="size || 11" :height="size || 11" viewBox="0 0 12 12" fill="none" class="v-icon">
    <rect x="1.5" y="1.5" width="9" height="9" rx="1.8" stroke="currentColor" stroke-width="1.6" fill="none"/>
  </svg>

  <!-- 17. Windows 标题栏: 关闭图标 (统一 1.6px 线宽 + 饱满圆角交叉线，与图2参考完全一致) -->
  <svg v-else-if="name === 'win-close'" :width="size || 11" :height="size || 11" viewBox="0 0 12 12" fill="none" class="v-icon">
    <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>
    <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>
  </svg>
  <!-- 18. 标签页锁定状态微型锁图标 (1:1 完美复刻 ssh8.py CustomTabCloseButton: 拱形挂钩锁梁 + 半透填充锁身 + 钥匙孔圆点) -->
  <svg v-else-if="name === 'lock'" :width="size || 15" :height="size || 15" viewBox="0 0 16 16" fill="none" class="v-icon icon-lock-tab">
    <!-- 1. 上部半圆锁梁弧线 (cx=8, cy=8, shackle from cx-3.2 to cx+3.2, y from 7 to 4.2 to arc) -->
    <path d="M4.8 7V4.2C4.8 2.43 6.23 1 8 1C9.77 1 11.2 2.43 11.2 4.2V7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
    <!-- 2. 下部圆角锁身 (半透明或实心由 CSS 填充，线条与背景配合) -->
    <rect x="3.5" y="6.8" width="9" height="7.2" rx="1.8" stroke="currentColor" stroke-width="1.2" fill="currentColor" fill-opacity="0.22"/>
    <!-- 3. 中心钥匙孔微型圆点 (直径 2px) -->
    <circle cx="8" cy="10" r="1" fill="currentColor"/>
  </svg>

  <!-- 19. 加号矢量图标 (+) -->
  <svg v-else-if="name === 'plus'" :width="size || 14" :height="size || 14" viewBox="0 0 16 16" fill="none" class="v-icon">
    <path d="M8 3V13M3 8H13" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
  </svg>

  <!-- 20. 减号矢量图标 (-) -->
  <svg v-else-if="name === 'minus'" :width="size || 14" :height="size || 14" viewBox="0 0 16 16" fill="none" class="v-icon">
    <path d="M3.5 8H12.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
  </svg>

  <!-- 21. 单向向上矢量箭头 (↑) -->
  <svg v-else-if="name === 'arrow-up'" :width="size || 14" :height="size || 14" viewBox="0 0 16 16" fill="none" class="v-icon">
    <path d="M8 12.5V3.5M4 7.5L8 3.5L12 7.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <!-- 22. 单向向下矢量箭头 (↓) -->
  <svg v-else-if="name === 'arrow-down'" :width="size || 14" :height="size || 14" viewBox="0 0 16 16" fill="none" class="v-icon">
    <path d="M8 3.5V12.5M4 8.5L8 12.5L12 8.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>

  <svg v-else-if="name === 'pin'" :width="size || 14" :height="size || 14" viewBox="0 0 24 24" fill="none" class="v-icon">
    <path d="M9.5 3.5H14.5L13.5 9.5L16.5 12.5V14.5H7.5V12.5L10.5 9.5L9.5 3.5Z" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" fill="currentColor" fill-opacity="0.15"/>
    <path d="M12 14.5V20.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>
  </svg>
  <!-- 23. 关闭/删除矢量叉号 (✕) -->
  <svg v-else-if="name === 'close'" :width="size || 12" :height="size || 12" viewBox="0 0 16 16" fill="none" class="v-icon">
    <path d="M4 4L12 12M12 4L4 12" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
  </svg>
  <svg v-else-if="name === 'split'" :width="size || 20" :height="size || 20" viewBox="0 0 24 24" fill="none" class="v-icon" xmlns="http://www.w3.org/2000/svg">
    <rect x="3" y="4" width="18" height="16" rx="2.5" stroke="currentColor" stroke-width="1.8" />
    <line x1="12" y1="4.5" x2="12" y2="19.5" stroke="currentColor" stroke-width="1.8" />
  </svg>
</template>

<script setup lang="ts">
defineProps<{
  name: "folder" | "chevron" | "host" | "file" | "settings" | "cloud" | "alert" | "bolt" | "moon" | "sun" | "bell" | "arrows" | "panel" | "doc" | "search" | "win-min" | "win-max" | "win-close" | "lock" | "plus" | "minus" | "arrow-up" | "arrow-down" | "close" | "fullscreen" | "pin" | "split" | "broadcast" | "refresh";
  size?: number | string;
  expanded?: boolean;
}>();
</script>

<style scoped>
.v-icon {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
  shape-rendering: geometricPrecision;
}
</style>
