/**
 * 配色主题预设（方案 1：主题 = 完整配色包，明暗各一版，强调色跟随主题主色）。
 *
 * CSS 变量覆盖在 app.css 中：
 *   [data-theme-preset="id"]                     → 暗色版
 *   [data-theme="light"][data-theme-preset="id"] → 亮色版
 */
export interface ThemeTerm {
  background: string;
  foreground: string;
  cursor: string;
}

export interface ThemeVariant {
  /** 主题主色（强调色自动跟随） */
  primary: string;
  /** 预览色点（背景 / 容器 / 主色） */
  swatch: [string, string, string];
  term: ThemeTerm;
}

export interface ThemePreset {
  id: string;
  name: string;
  dark: ThemeVariant;
  light: ThemeVariant;
}

export const THEME_PRESETS: ThemePreset[] = [
  {
    id: "default",
    name: "经典",
    dark: {
      primary: "#A8C7FA",
      swatch: ["#151517", "#202024", "#A8C7FA"],
      term: { background: "#0A0B0E", foreground: "#81C995", cursor: "#81C995" },
    },
    light: {
      primary: "#2057A6",
      swatch: ["#ECECEE", "#FFFFFF", "#2057A6"],
      term: { background: "#F4F5F7", foreground: "#111318", cursor: "#111318" },
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    dark: {
      primary: "#bd93f9",
      swatch: ["#21222c", "#44475a", "#bd93f9"],
      term: { background: "#282a36", foreground: "#f8f8f2", cursor: "#bd93f9" },
    },
    light: {
      primary: "#7c5cbf",
      swatch: ["#f2f0fb", "#ffffff", "#7c5cbf"],
      term: { background: "#fbf9ff", foreground: "#2d2a3e", cursor: "#7c5cbf" },
    },
  },
  {
    id: "one-dark",
    name: "One Dark",
    dark: {
      primary: "#61afef",
      swatch: ["#282c34", "#3b4048", "#61afef"],
      term: { background: "#282c34", foreground: "#abb2bf", cursor: "#528bff" },
    },
    light: {
      primary: "#2d6fbf",
      swatch: ["#f3f5f8", "#ffffff", "#2d6fbf"],
      term: { background: "#fafbfc", foreground: "#30353d", cursor: "#2d6fbf" },
    },
  },
  {
    id: "nord",
    name: "Nord",
    dark: {
      primary: "#88c0d0",
      swatch: ["#2e3440", "#3b4252", "#88c0d0"],
      term: { background: "#2e3440", foreground: "#d8dee9", cursor: "#88c0d0" },
    },
    light: {
      primary: "#2e7d8f",
      swatch: ["#eef2f6", "#ffffff", "#2e7d8f"],
      term: { background: "#f8fafc", foreground: "#2e3440", cursor: "#2e7d8f" },
    },
  },
  {
    id: "solarized",
    name: "Solarized",
    dark: {
      primary: "#268bd2",
      swatch: ["#002b36", "#073642", "#268bd2"],
      term: { background: "#002b36", foreground: "#93a1a1", cursor: "#268bd2" },
    },
    light: {
      primary: "#268bd2",
      swatch: ["#fdf6e3", "#eee8d5", "#268bd2"],
      term: { background: "#fdf6e3", foreground: "#586e75", cursor: "#268bd2" },
    },
  },
];

export function findThemePreset(id: string | undefined | null): ThemePreset {
  return THEME_PRESETS.find((t) => t.id === id) || THEME_PRESETS[0];
}

/** 取主题在指定明暗下的变体。 */
export function themeVariant(preset: ThemePreset, dark: boolean): ThemeVariant {
  return dark ? preset.dark : preset.light;
}
