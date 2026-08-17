export interface BibleFile {
  name: string;
  path: string;
}

export interface Character {
  name: string;
  path: string;
}

export interface BibleMeta {
  files: BibleFile[];
  characters: Character[];
}

export interface Chapter {
  file: string;
  path: string;
  title: string;
  bytes: number;
  chars: number;
  head: string;
  modifiedMs: number;
}

export interface NovelState {
  title: string;
  currentChapter: string;
  worldDate: string;
  pov: string;
  nextHook: string;
  foreshadowingOpen: number;
}

export interface NovelSummary {
  ok: boolean;
  root: string;
  title: string;
  state: NovelState;
  bible: BibleMeta;
  chapters: Chapter[];
  recentChapters: Chapter[];
}

export function fmtChars(n: number): string {
  if (n >= 10000) return (n / 10000).toFixed(1) + " 万";
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return String(n);
}

export function fmtDate(ms: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

// ---------------------------------------------------------------------------
// 新建小说向导（4 步表单）
// ---------------------------------------------------------------------------

export type Genre =
  | "玄幻"
  | "都市"
  | "科幻"
  | "言情"
  | "历史"
  | "悬疑"
  | "武侠"
  | "其他";

export type Tone = "轻松" | "严肃" | "黑暗" | "爽文" | "治愈";

export const GENRES: Genre[] = ["玄幻", "都市", "科幻", "言情", "历史", "悬疑", "武侠", "其他"];
export const TONES: Tone[] = ["轻松", "严肃", "黑暗", "爽文", "治愈"];
export const POV_MODES = ["第一人称", "第三人称", "多重视角"] as const;
export type PovMode = (typeof POV_MODES)[number];

export interface NovelBrief {
  parent: string;          // 父目录
  name: string;            // 目录名
  title: string;           // 标题
  povMode: PovMode;
  povCharacter: string;
  genre: Genre;
  tone: Tone;
  era: string;             // "架空" / "现代" / "未来 2187" / "唐贞观" / 自填
  targetWordsWan: number;  // 万字
  volumes: number;
  chaptersPerVolume: number;
  coreConflict: string;    // 一句话核心矛盾
  heroSituation: string;   // 一句话主角处境
  heroDesire: string;      // 一句话主角欲望
  openingHook: string;     // 一句话第一幕核心冲突
}

export function presetEras(): string[] {
  return ["架空", "现代都市", "近未来", "远古", "中世纪", "唐贞观", "宋汴京", "清末民初", "民国"];
}

export function deriveProjectName(title: string): string {
  // 让目录名安全：去标点 / 空格 → 下划线，纯 ASCII/中文保留
  const cleaned = title
    .trim()
    .replace(/[\\\/:*?"<>|]/g, "")
    .replace(/\s+/g, "-");
  return cleaned || "未命名小说";
}

export function deriveSingleChapterWords(brief: NovelBrief): number {
  const totalWords = brief.targetWordsWan * 10_000;
  const totalChapters = brief.volumes * brief.chaptersPerVolume;
  if (totalChapters <= 0) return 0;
  return Math.round(totalWords / totalChapters);
}

// ---------------------------------------------------------------------------
// 路径诊断 — 前端用来显示"打开失败"时的友好提示 + 动作按钮
// ---------------------------------------------------------------------------

export type Probe =
  | { kind: "missing"; path: string; parent: string; suggestedName: string }
  | { kind: "fileNotDir"; path: string; parent: string; suggestedName: string }
  | { kind: "emptyDir"; path: string; parent: string; suggestedName: string }
  | { kind: "nonEmptyDir"; path: string; parent: string; suggestedName: string; sample: string[] }
  | { kind: "novelSubdir"; path: string; parent: string; suggestedName: string; root: string }
  | { kind: "novelRoot"; path: string; parent: string; suggestedName: string };
