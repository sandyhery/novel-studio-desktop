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
  /** 单章建议字数目标（0 = 未知，不显示达标 banner） */
  chapterTargetWords: number;
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

// ---------------------------------------------------------------------------
// 抉择点 / 剧情分支
// ---------------------------------------------------------------------------

export type ChoiceWeight = "critical" | "major" | "minor" | "flavor";

export interface ChoiceOption {
  id: string;
  label: string;
  previewHint: string; // 1-2 句"如果选 X 接下来会怎样"
}

export interface DecisionRecord {
  by: "human" | "ai";
  optionId: string;
  decidedAt: string; // ISO 8601
  note: string | null;
}

export interface ChoicePoint {
  id: string;
  weight: ChoiceWeight;
  afterChapter: string;
  prompt: string;
  options: ChoiceOption[];
  decided: DecisionRecord | null;
}

export interface ChoicePointsView {
  root: string;
  aiNovelDirExists: boolean;
  points: ChoicePoint[];
  decidedCount: number;
  pendingCount: number;
}

/** 决定一个抉择点用的 args（与 Rust 端的 DecideChoiceArgs 对应）。 */
export interface DecideChoiceArgs {
  root: string;
  pointId: string;
  optionId: string;
  by: "human" | "ai";
  note?: string | null;
}

/** 一个 weight 的中文副本 */
export function weightLabel(w: ChoiceWeight): string {
  switch (w) {
    case "critical": return "重大转折";
    case "major": return "重要选择";
    case "minor": return "小决定";
    case "flavor": return "调味";
  }
}

/** weight 对应配色：使用现有的 danger/warn 之类 */
export function weightTone(w: ChoiceWeight): "danger" | "warn" | "muted" {
  switch (w) {
    case "critical": return "danger";
    case "major": return "warn";
    case "minor":
    case "flavor":
      return "muted";
  }
}

// ---------------------------------------------------------------------------
// DSH agent 集成 — AI 写章节
// ---------------------------------------------------------------------------

export interface AiWriteChapterArgs {
  root: string;
  instruction?: string | null;
  sessionId?: string | null;
  timeoutSecs?: number;
  port?: number;
}

export interface AiWriteChapterResult {
  ok: boolean;
  text: string;
  choiceRequest: {
    prompt: string;
    options: Array<{ id: string; label: string; previewHint: string }>;
  } | null;
  sessionId: string | null;
  savedTo: string | null;
  error: string | null;
}

export interface AiReconcileBibleArgs {
  root: string;
  chapterFile?: string | null;
  decision?: string | null;
  sessionId?: string | null;
  timeoutSecs?: number;
  port?: number;
}

export interface AiReconcileBibleResult {
  ok: boolean;
  text: string;
  sessionId: string | null;
  error: string | null;
}

// ---------------------------------------------------------------------------
// 剧情树（Story Spine）
// ---------------------------------------------------------------------------

export interface SpineNode {
  kind: "chapter" | "choice" | "branch";
  id: string;
  title: string;
  weight: string | null;
  decided: string | null;
  chars: number | null;
}

export interface StorySpine {
  ok: boolean;
  root: string;
  nodes: SpineNode[];
  branches: Array<[string, SpineNode[]]>;
}

// ---------------------------------------------------------------------------
// AI 审核员
// ---------------------------------------------------------------------------

export interface ReviewIssue {
  severity: "critical" | "major" | "minor" | "info";
  location: string;
  issue: string;
  suggestion: string;
}

export interface ReviewCategory {
  label: string;
  issues: ReviewIssue[];
}

export interface ReviewReport {
  ok: boolean;
  chapterFile: string;
  summary: string;
  verdict: "pass" | "revise";
  categories: Record<string, ReviewCategory>;
  sessionId: string | null;
  error: string | null;
}

export interface AiReviewChapterArgs {
  root: string;
  chapterFile?: string | null;
  sessionId?: string | null;
  timeoutSecs?: number;
  port?: number;
}

export interface AiReviseChapterArgs {
  root: string;
  chapterFile: string;
  reportJson?: string | null;
  sessionId?: string | null;
  timeoutSecs?: number;
  port?: number;
}

export interface AiReviseChapterResult {
  ok: boolean;
  revisedText: string;
  savedTo: string | null;
  sessionId: string | null;
  error: string | null;
}

export interface AiFullPipelineArgs {
  root: string;
  instruction?: string | null;
  autoRevise?: boolean;
  autoReconcile?: boolean;
  sessionId?: string | null;
  timeoutSecs?: number;
  port?: number;
}

export interface AiFullPipelineResult {
  ok: boolean;
  stage: "write_done" | "reviewed" | "revised" | "done" | "done_revised" | "choice_pending" | "error";
  chapterFile: string | null;
  reviewSummary: string | null;
  verdict: string | null;
  finalText: string | null;
  reconcileNote: string | null;
  sessionId: string | null;
  error: string | null;
}

/** 审核问题类别顺序 */
export const REVIEW_CATEGORY_ORDER = ["A", "B", "C", "D", "P"] as const;

// ---------------------------------------------------------------------------
// Git 快照 / 里程碑自动提交
// ---------------------------------------------------------------------------

export interface GitCommitResult {
  repoExists: boolean;
  committed: boolean;
  message: string;
  hash: string | null;
  summary: string;
}

export interface GitInitResult {
  ok: boolean;
  repoExists: boolean;
  summary: string;
}

// ---------------------------------------------------------------------------
// 双向打通（#8）：文件指纹 + 变更检测 + 写冲突
// ---------------------------------------------------------------------------

/** 文件指纹（mtime 毫秒 + 字节数），用于判断「磁盘版本 vs 内存草稿」是否一致。 */
export interface Fingerprint {
  mtimeMs: number;
  size: number;
}

export interface WriteChapterResult {
  ok: boolean;
  conflict: boolean;
  mtimeMs: number;
  size: number;
}

export interface FileStamp {
  /** 相对小说根的路径，如 "chapters/ch01.md" / "bible/timeline.md" / "state.yml" */
  path: string;
  mtimeMs: number;
  size: number;
}

export interface ReadChangesResult {
  changed: boolean;
  stamps: FileStamp[];
}

// ---------------------------------------------------------------------------
// DSH Web 登录
// ---------------------------------------------------------------------------

export interface DshLoginResult {
  ok: boolean;
  mfaRequired: boolean;
  mfaToken: string | null;
  message: string;
}
