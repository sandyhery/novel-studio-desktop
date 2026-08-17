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
