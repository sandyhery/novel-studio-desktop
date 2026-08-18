import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { BibleMeta, Chapter, FileStamp, Fingerprint, GitCommitResult, GitInitResult, NovelState, NovelSummary, Probe, ReadChangesResult, WriteChapterResult } from "./types";
import { fmtChars, fmtDate } from "./types";
import CreateNovelModal from "./CreateNovelModal";
import StoryboardPanel from "./StoryboardPanel";
import AiWritePanel from "./AiWritePanel";
import ReviewPanel from "./ReviewPanel";
import MarkdownView from "./MarkdownView";
import DshLoginModal from "./DshLoginModal";

type Panel = "overview" | "chapters" | "bible" | "editor" | "storyboard" | "aiwrite" | "review";

interface BibleSelection {
  /** "timeline" / "foreshadowing" / "characters/林楚" 这种带不带路径的 key */
  file: string;
  display: string;
}

/** 从 localStorage 读一个数值型偏好，越界/损坏时退回默认值。 */
function readPrefNumber(key: string, fallback: number, min: number, max: number): number {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v >= min && v <= max ? v : fallback;
  } catch {
    return fallback;
  }
}

export default function App() {
  const [root, setRoot] = useState<string | null>(null);
  const [summary, setSummary] = useState<NovelSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [panel, setPanel] = useState<Panel>("overview");
  const [selectedBible, setSelectedBible] = useState<BibleSelection | null>(null);
  const [bibleContent, setBibleContent] = useState<string>("");
  const [bibleError, setBibleError] = useState<string | null>(null);
  const [editingChapter, setEditingChapter] = useState<Chapter | null>(null);
  const [chapterDraft, setChapterDraft] = useState<string>("");
  const [chapterSaved, setChapterSaved] = useState<boolean>(true);
  const [saving, setSaving] = useState(false);
  const [baseFingerprint, setBaseFingerprint] = useState<Fingerprint | null>(null);
  const [conflict, setConflict] = useState<{ mtimeMs: number; size: number } | null>(null);
  const [externallyModified, setExternallyModified] = useState(false);

  // 供轮询 / 回调读取最新值的 ref（避免闭包过期）。
  const stampsRef = useRef<FileStamp[] | null>(null);
  const editingChapterRef = useRef<Chapter | null>(null);
  const baseFingerprintRef = useRef<Fingerprint | null>(null);
  const busyRef = useRef(false);
  const savingRef = useRef(false);
  const [showCreate, setShowCreate] = useState(false);
  const [dshSessionId, setDshSessionId] = useState<string | null>(null);
  const [probe, setProbe] = useState<Probe | null>(null);
  const [probeFor, setProbeFor] = useState<string | null>(null);
  const [briefInit, setBriefInit] = useState<{ parent?: string | null; name?: string } | null>(null);
  const [readMode, setReadMode] = useState(false);
  const [immersed, setImmersed] = useState(false);
  const [fontSize, setFontSize] = useState<number>(() => readPrefNumber("novelStudio.fontSize", 16, 12, 32));
  const [lineHeight, setLineHeight] = useState<number>(() => readPrefNumber("novelStudio.lineHeight", 1.85, 1.2, 3));
  const [milestoneEvery, setMilestoneEvery] = useState<number>(() => readPrefNumber("novelStudio.milestoneEvery", 5, 0, 100));
  const [gitNotice, setGitNotice] = useState<{ text: string; tone: "ok" | "warn" | "error" } | null>(null);
  const [dshLoggedIn, setDshLoggedIn] = useState(false);
  const [showDshLogin, setShowDshLogin] = useState(false);

  const changeFontSize = useCallback((n: number) => {
    setFontSize(n);
    try { localStorage.setItem("novelStudio.fontSize", String(n)); } catch { /* ignore */ }
  }, []);

  const changeLineHeight = useCallback((n: number) => {
    setLineHeight(n);
    try { localStorage.setItem("novelStudio.lineHeight", String(n)); } catch { /* ignore */ }
  }, []);

  const changeMilestoneEvery = useCallback((n: number) => {
    setMilestoneEvery(n);
    try { localStorage.setItem("novelStudio.milestoneEvery", String(n)); } catch { /* ignore */ }
  }, []);

  const toggleImmerse = useCallback(async () => {
    const next = !immersed;
    setImmersed(next);
    try {
      await getCurrentWindow().setFullscreen(next);
    } catch {
      // OS 全屏失败也不影响：CSS 沉浸层仍会隐藏顶部/侧栏
    }
  }, [immersed]);

  const doCommit = useCallback(
    async (message?: string): Promise<GitCommitResult | null> => {
      const r = summary?.root;
      if (!r) return null;
      try {
        const res = await invoke<GitCommitResult>("git_commit", {
          root: r,
          message: message ?? null,
        });
        setGitNotice({
          text: res.summary,
          tone: res.committed ? "ok" : "warn",
        });
        return res;
      } catch (e) {
        setGitNotice({ text: `提交失败：${String(e)}`, tone: "error" });
        return null;
      }
    },
    [summary?.root],
  );

  const doInit = useCallback(async () => {
    const r = summary?.root;
    if (!r) return;
    try {
      const res = await invoke<GitInitResult>("git_init", { root: r });
      setGitNotice({ text: res.summary, tone: res.ok ? "ok" : "error" });
      if (res.ok && res.repoExists) {
        await doCommit("初始化快照");
      }
    } catch (e) {
      setGitNotice({ text: `初始化失败：${String(e)}`, tone: "error" });
    }
  }, [summary?.root, doCommit]);

  // 里程碑自动提交：新增一章后，若章节数达到 N 的倍数，自动 git 快照。
  const prevChapterCountRef = useRef<number | null>(null);
  const lastRootRef = useRef<string | null>(null);
  useEffect(() => {
    if (!summary) return; // loadSummary 中间态（summary=null）不动 refs
    if (summary.root !== lastRootRef.current) {
      lastRootRef.current = summary.root;
      prevChapterCountRef.current = summary.chapters.length;
      return;
    }
    const count = summary.chapters.length;
    const prev = prevChapterCountRef.current ?? 0;
    prevChapterCountRef.current = count;
    if (count <= prev) return; // 没新增章节
    if (milestoneEvery > 0 && count % milestoneEvery === 0) {
      void doCommit(`里程碑：完成第 ${count} 章`);
    }
  }, [summary, milestoneEvery, doCommit]);

  const runProbe = useCallback(async (path: string) => {
    try {
      const p = await invoke<Probe>("probe_directory", { path });
      setProbe(p);
      setProbeFor(path);
    } catch (e) {
      setProbe(null);
      setProbeFor(path);
      setError(String(e));
    }
  }, []);

  const loadSummary = useCallback(
    async (rootPath: string): Promise<NovelSummary | null> => {
      setBusy(true);
      setError(null);
      setSummary(null);
      setProbe(null);
      setProbeFor(rootPath);
      // 重载后让轮询重新播种快照，避免把「自己刚写的改动」误判为外部变化
      stampsRef.current = null;
      try {
        const s = await invoke<NovelSummary>("read_summary", { root: rootPath });
        if (!s.ok) {
          setError("read_summary 返回 ok=false");
          await runProbe(rootPath);
          return null;
        }
        setSummary(s);
        setRoot(s.root);
        setSelectedBible(null);
        setBibleContent("");
        return s;
      } catch (e) {
        setError(String(e));
        await runProbe(rootPath);
        return null;
      } finally {
        setBusy(false);
      }
    },
    [runProbe],
  );

  const pickRoot = useCallback(async () => {
    try {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) {
        await loadSummary(picked);
      }
    } catch (e) {
      setError(String(e));
    }
  }, [loadSummary]);

  useEffect(() => {
    // 应用启动后自动尝试上次的根目录，否则提示用户挑选
    // 这里不强求自动加载，避免误打开用户其它路径
    pickRoot();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 启动时检查 DSH Web 登录态（是否有保存的 session cookie）
  useEffect(() => {
    let cancelled = false;
    invoke<boolean>("dsh_login_status")
      .then((ok) => {
        if (!cancelled) setDshLoggedIn(ok);
      })
      .catch(() => {
        if (!cancelled) setDshLoggedIn(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const openBible = useCallback(async (sel: BibleSelection) => {
    if (!root) return;
    setSelectedBible(sel);
    setBibleError(null);
    setBibleContent("");
    try {
      const text = await invoke<string>("read_bible", { root, file: sel.file });
      setBibleContent(text);
    } catch (e) {
      setBibleError(String(e));
    }
  }, [root]);

  const openChapter = useCallback(async (chap: Chapter) => {
    if (!root) return;
    setEditingChapter(chap);
    setChapterDraft("");
    setChapterSaved(true);
    setSaving(false);
    setConflict(null);
    setExternallyModified(false);
    setBaseFingerprint({ mtimeMs: chap.modifiedMs, size: chap.bytes });
    try {
      const text = await invoke<string>("read_chapter", { root, file: chap.file });
      setChapterDraft(text);
    } catch (e) {
      setError(String(e));
    }
  }, [root]);

  const saveChapter = useCallback(async (force = false) => {
    if (!root || !editingChapter) return;
    setSaving(true);
    try {
      const res = await invoke<WriteChapterResult>("write_chapter", {
        args: {
          root,
          file: editingChapter.file,
          content: chapterDraft,
          baseFingerprint: force ? null : baseFingerprint,
        },
      });
      if (!res.ok && res.conflict) {
        setConflict({ mtimeMs: res.mtimeMs, size: res.size });
        return;
      }
      setChapterSaved(true);
      setBaseFingerprint({ mtimeMs: res.mtimeMs, size: res.size });
      setConflict(null);
      setExternallyModified(false);
      await loadSummary(root);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, [root, editingChapter, chapterDraft, baseFingerprint, loadSummary]);

  // 冲突时「载入磁盘版本」：重读正文 + 刷新该章指纹（放弃本地草稿）。
  const reloadChapterFromDisk = useCallback(async () => {
    if (!root || !editingChapter) return;
    try {
      const text = await invoke<string>("read_chapter", { root, file: editingChapter.file });
      setChapterDraft(text);
      setChapterSaved(true);
      setConflict(null);
      setExternallyModified(false);
      const s = await loadSummary(root);
      if (s) {
        const fresh = s.chapters.find((c) => c.file === editingChapter.file);
        if (fresh) {
          setEditingChapter(fresh);
          setBaseFingerprint({ mtimeMs: fresh.modifiedMs, size: fresh.bytes });
        }
      }
    } catch (e) {
      setError(String(e));
    }
  }, [root, editingChapter, loadSummary]);

  // —— 自动刷新（A）：2s 轮询廉价变更检测，有外部改动才刷看板 ——
  useEffect(() => { editingChapterRef.current = editingChapter; }, [editingChapter]);
  useEffect(() => { baseFingerprintRef.current = baseFingerprint; }, [baseFingerprint]);
  useEffect(() => { busyRef.current = busy; }, [busy]);
  useEffect(() => { savingRef.current = saving; }, [saving]);

  useEffect(() => {
    if (!root) return;
    let cancelled = false;
    const tick = async () => {
      if (document.hidden || busyRef.current || savingRef.current) return;
      try {
        const res = await invoke<ReadChangesResult>("read_changes", {
          args: { root, lastSeen: stampsRef.current },
        });
        if (cancelled) return;
        stampsRef.current = res.stamps;

        if (res.changed) {
          void loadSummary(root);
        }

        // 正在编辑的章若被外部改动 → 提示（不自动覆盖草稿）
        const editing = editingChapterRef.current;
        const base = baseFingerprintRef.current;
        if (editing && base) {
          const ep = `chapters/${editing.file}`;
          const stamp = res.stamps.find((s) => s.path === ep);
          if (stamp && (stamp.mtimeMs !== base.mtimeMs || stamp.size !== base.size)) {
            setExternallyModified(true);
          }
        }
      } catch {
        // 轮询失败静默忽略（项目刚卸载等）
      }
    };
    const id = setInterval(tick, 2000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [root, loadSummary]);

  const totalChars = useMemo(
    () => summary?.chapters.reduce((sum, c) => sum + c.chars, 0) ?? 0,
    [summary],
  );

  const targetWords = summary?.state.chapterTargetWords ?? 0;

  const editorPanel = (
    <EditorPanel
      chapter={editingChapter}
      draft={chapterDraft}
      saved={chapterSaved}
      saving={saving}
      externallyModified={externallyModified}
      onReloadDisk={reloadChapterFromDisk}
      onChange={(v) => {
        setChapterDraft(v);
        setChapterSaved(false);
      }}
      onSave={() => saveChapter(false)}
      readMode={readMode}
      onToggleReadMode={() => setReadMode((v) => !v)}
      immersed={immersed}
      onToggleImmerse={toggleImmerse}
      fontSize={fontSize}
      onFontSize={changeFontSize}
      lineHeight={lineHeight}
      onLineHeight={changeLineHeight}
      targetWords={targetWords}
    />
  );

  if (immersed && editingChapter) {
    return <div className="immersive">{editorPanel}</div>;
  }

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="logo">📖</span>
          <div>
            <h1>小说工作台</h1>
            <p className="subtitle">agt novel-init 项目专属客户端 · 本地读取 · 不依赖 DSH Web</p>
          </div>
        </div>
        <div className="topbar-right">
          {summary && (
            <span className="meta">
              《{summary.title}》 · {summary.chapters.length} 章 · {fmtChars(totalChars)} 字
            </span>
          )}
          <button className="btn primary" onClick={pickRoot} disabled={busy}>
            {busy ? "加载中…" : root ? "切项目" : "打开小说项目"}
          </button>
          <button className="btn" onClick={() => setShowCreate(true)} disabled={busy}>
            ✨ 新建
          </button>
          <button
            className={`btn ${dshLoggedIn ? "" : "primary"}`}
            onClick={() => {
              if (dshLoggedIn) {
                invoke<boolean>("dsh_logout")
                  .then(() => setDshLoggedIn(false))
                  .catch(() => {});
              } else {
                setShowDshLogin(true);
              }
            }}
            title={dshLoggedIn ? "已登录 DSH Web，点击登出" : "登录 DSH Web（AI 写章节/审核需要）"}
          >
            {dshLoggedIn ? "🔓 DSH 已登录" : "🔐 登录 DSH"}
          </button>
        </div>
      </header>

      <div className="layout">
        <aside className="sidebar">
          <button
            className={`nav-item ${panel === "overview" ? "active" : ""}`}
            onClick={() => setPanel("overview")}
            disabled={!summary}
          >
            <span className="nav-icon">🧭</span>
            <span>项目概览</span>
          </button>
          <button
            className={`nav-item ${panel === "chapters" ? "active" : ""}`}
            onClick={() => setPanel("chapters")}
            disabled={!summary}
          >
            <span className="nav-icon">📚</span>
            <span>章节看板</span>
          </button>
          <button
            className={`nav-item ${panel === "bible" ? "active" : ""}`}
            onClick={() => setPanel("bible")}
            disabled={!summary}
          >
            <span className="nav-icon">📜</span>
            <span>世界圣经</span>
          </button>
          <button
            className={`nav-item ${panel === "editor" ? "active" : ""}`}
            onClick={() => setPanel("editor")}
            disabled={!editingChapter}
          >
            <span className="nav-icon">✍️</span>
            <span>章节编辑器</span>
            {!chapterSaved && <span className="dot warn" title="有未保存的修改" />}
          </button>
          <button
            className={`nav-item ${panel === "storyboard" ? "active" : ""}`}
            onClick={() => setPanel("storyboard")}
            disabled={!summary}
          >
            <span className="nav-icon">🎯</span>
            <span>剧情走向</span>
            {summary && summary.state.foreshadowingOpen > 0 && (
              <span className="muted small" style={{ marginLeft: 4 }}>
                · 可定 {summary.state.foreshadowingOpen}
              </span>
            )}
          </button>
          <button
            className={`nav-item ${panel === "aiwrite" ? "active" : ""}`}
            onClick={() => setPanel("aiwrite")}
            disabled={!summary}
          >
            <span className="nav-icon">🤖</span>
            <span>AI 写章节</span>
          </button>
          <button
            className={`nav-item ${panel === "review" ? "active" : ""}`}
            onClick={() => setPanel("review")}
            disabled={!summary}
          >
            <span className="nav-icon">🛡️</span>
            <span>AI 审核</span>
          </button>

          {summary && (
            <div className="section">
              <div className="label">伏笔</div>
              <div className="badge-row">
                <span className="badge">待收 {summary.state.foreshadowingOpen}</span>
              </div>
            </div>
          )}

          {summary && (
            <div className="section">
              <div className="label">最新章节</div>
              <ul className="recent">
                {summary.recentChapters.slice(0, 5).map((c) => (
                  <li key={c.file}>
                    <button
                      className="recent-item"
                      onClick={() => {
                        openChapter(c);
                        setPanel("editor");
                      }}
                      title={c.path}
                    >
                      <span className="recent-title">{c.title}</span>
                      <span className="muted small">{fmtDate(c.modifiedMs)}</span>
                    </button>
                  </li>
                ))}
                {summary.recentChapters.length === 0 && (
                  <li className="muted small">暂无章节</li>
                )}
              </ul>
            </div>
          )}
        </aside>

        <main className="main">
          {error && <div className="error-banner">{error}</div>}
          {gitNotice && (
            <div className={`git-notice ${gitNotice.tone}`}>
              <span>{gitNotice.tone === "ok" ? "✅" : gitNotice.tone === "error" ? "❌" : "ℹ️"}</span>
              <span className="git-notice-text">{gitNotice.text}</span>
              <button className="btn ghost" onClick={() => setGitNotice(null)} title="关闭">
                ✕
              </button>
            </div>
          )}

          {!summary && !probe && !error && (
            <div className="empty">
              <h2>请选择小说项目目录</h2>
              <p className="muted">
                必须是 <code>agt novel-init</code> 生成的根目录（包含 <code>state.yml</code> 与 <code>bible/</code>）。
              </p>
              <button className="btn primary" onClick={pickRoot}>
                选目录
              </button>
              <p className="muted small" style={{ marginTop: 16 }}>
                或者
              </p>
              <button className="btn" onClick={() => setShowCreate(true)}>
                ✨ 从零新建一本
              </button>
            </div>
          )}

          {!summary && probe && (
            <ProbeCard
              probe={probe}
              chosenPath={probeFor}
              onChooseOther={pickRoot}
              onInitInPlace={() => {
                // 把向导预填为"在父目录里新建一个同名子目录"
                setShowCreate(true);
                const parent = probe?.parent ?? "";
                const name = probe?.suggestedName ?? "";
                setBriefInit({ parent, name });
              }}
              onInitAtParent={() => {
                setShowCreate(true);
                const parent = probe?.parent ?? "";
                setBriefInit({ parent });
              }}
              onInitAtRoot={(root) => {
                setShowCreate(true);
                setBriefInit({ parent: root });
              }}
            />
          )}

          {summary && panel === "overview" && (
            <OverviewPanel
              state={summary.state}
              bible={summary.bible}
              recent={summary.recentChapters}
              totalChars={totalChars}
              chapterCount={summary.chapters.length}
              milestoneEvery={milestoneEvery}
              onMilestoneEvery={changeMilestoneEvery}
              onCommit={() => doCommit()}
              onInit={doInit}
              openChapter={(c) => {
                openChapter(c);
                setPanel("editor");
              }}
              openBible={(file, display) => {
                openBible({ file, display });
                setPanel("bible");
              }}
            />
          )}

          {summary && panel === "chapters" && (
            <ChaptersPanel
              chapters={summary.chapters}
              onOpen={(c) => {
                openChapter(c);
                setPanel("editor");
              }}
            />
          )}

          {summary && panel === "bible" && (
            <BiblePanel
              meta={summary.bible}
              selected={selectedBible}
              content={bibleContent}
              error={bibleError}
              onSelect={openBible}
            />
          )}

          {summary && panel === "storyboard" && (
            <StoryboardPanel root={summary.root} />
          )}

          {summary && panel === "aiwrite" && (
            <AiWritePanel
              root={summary.root}
              sessionId={dshSessionId}
              onSessionId={setDshSessionId}
              onSaved={() => loadSummary(summary.root)}
              dshLoggedIn={dshLoggedIn}
              onLogin={() => setShowDshLogin(true)}
            />
          )}

          {summary && panel === "review" && (
            <ReviewPanel
              root={summary.root}
              sessionId={dshSessionId}
              onSessionId={setDshSessionId}
              onSaved={() => loadSummary(summary.root)}
              dshLoggedIn={dshLoggedIn}
              onLogin={() => setShowDshLogin(true)}
            />
          )}

          {summary && panel === "editor" && editorPanel}
        </main>
      </div>

      {showCreate && (
        <CreateNovelModal
          initialParent={briefInit?.parent ?? root}
          initialName={briefInit?.name ?? ""}
          onClose={() => {
            setShowCreate(false);
            setBriefInit(null);
          }}
          onCreated={async (newRoot) => {
            setShowCreate(false);
            setBriefInit(null);
            await loadSummary(newRoot);
          }}
        />
      )}

      {showDshLogin && (
        <DshLoginModal
          onClose={() => setShowDshLogin(false)}
          onLoggedIn={() => setDshLoggedIn(true)}
        />
      )}

      {conflict && editingChapter && (
        <div className="modal-overlay">
          <div className="modal choice-modal">
            <div className="modal-header">
              <h2>⚠️ 磁盘版本已变</h2>
              <button className="btn ghost" onClick={() => setConflict(null)}>✕</button>
            </div>
            <div className="modal-body">
              <p className="choice-prompt">
                《{editingChapter.title}》在你编辑期间被别处修改了（DSH agent / 插件 / 其它编辑器）。
                直接保存会覆盖对方的改动，请选择：
              </p>
              <div className="options-list large">
                <button className="option pickable" onClick={() => void reloadChapterFromDisk()}>
                  <div className="option-head">
                    <span className="option-label">📥 载入磁盘版本</span>
                  </div>
                  <p className="option-hint">放弃当前草稿，读入磁盘上的最新内容</p>
                </button>
                <button className="option pickable" onClick={() => void saveChapter(true)}>
                  <div className="option-head">
                    <span className="option-label">💾 仍用我的版本覆盖</span>
                  </div>
                  <p className="option-hint">保留我的草稿并覆盖磁盘（对方的改动会丢失）</p>
                </button>
                <button className="option pickable" onClick={() => setConflict(null)}>
                  <div className="option-head">
                    <span className="option-label">↩ 取消</span>
                  </div>
                  <p className="option-hint">继续编辑，稍后再处理</p>
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

function ProbeCard(props: {
  probe: Probe;
  chosenPath: string | null;
  onChooseOther: () => void;
  onInitInPlace: (path: string) => void;
  onInitAtParent: () => void;
  onInitAtRoot: (root: string) => void;
}) {
  const { probe, chosenPath, onChooseOther, onInitInPlace, onInitAtParent, onInitAtRoot } = props;

  let title = "未找到小说项目";
  let body = "";
  let action: React.ReactNode = null;

  switch (probe.kind) {
    case "missing":
      title = "路径不存在";
      body = `目录 ${probe.path} 还没创建。`;
      action = (
        <button className="btn primary" onClick={() => onInitInPlace(probe.path)}>
          ✨ 在这里创建小说
        </button>
      );
      break;
    case "fileNotDir":
      title = "这是个文件不是目录";
      body = `${probe.path} 是文件。请选别的目录。`;
      action = (
        <button className="btn" onClick={onChooseOther}>选别的目录</button>
      );
      break;
    case "emptyDir":
      title = "这是空目录";
      body = `${probe.path} 是空目录。原地跑 agt novel-init 就能创建项目。`;
      action = (
        <>
          <button className="btn primary" onClick={() => onInitInPlace(probe.path)}>
            ✨ 原地创建小说（项目名 = 「${probe.suggestedName}」）
          </button>
          <button className="btn" onClick={onChooseOther}>选别的</button>
        </>
      );
      break;
    case "nonEmptyDir":
      title = "不是 novel 项目";
      body = `${probe.path} 不是 novel-init 项目，里面有别的东西：${probe.sample.join("、")}`;
      action = (
        <>
          <button className="btn primary" onClick={onInitAtParent}>
            ✨ 在父目录新建一个子项目
          </button>
          <button className="btn" onClick={onChooseOther}>选别的</button>
        </>
      );
      break;
    case "novelSubdir":
      title = "这是 novel 项目的子目录";
      body = `选的是 ${probe.path}，但它在项目 ${probe.root} 里面。`;
      action = (
        <>
          <button className="btn primary" onClick={() => onInitAtRoot(probe.root)}>
            ↗ 打开整个项目（${probe.root}）
          </button>
          <button className="btn" onClick={onChooseOther}>选别的</button>
        </>
      );
      break;
    case "novelRoot":
      title = "是 novel 项目";
      body = `${probe.path} 正是项目根，但读取失败了。`;
      break;
  }

  return (
    <div className="panel">
      <div className="diagnostic">
        <h2>{title}</h2>
        <p className="muted">{body}</p>
        {chosenPath && (
          <p className="muted small">
            你选的路径：<code>{chosenPath}</code>
          </p>
        )}
        <div className="diagnostic-actions">{action}</div>
      </div>
    </div>
  );
}

function OverviewPanel(props: {
  state: NovelState;
  bible: BibleMeta;
  recent: Chapter[];
  totalChars: number;
  chapterCount: number;
  milestoneEvery: number;
  onMilestoneEvery: (n: number) => void;
  onCommit: () => void;
  onInit: () => void;
  openChapter: (c: Chapter) => void;
  openBible: (file: string, display: string) => void;
}) {
  const {
    state,
    bible,
    recent,
    totalChars,
    chapterCount,
    milestoneEvery,
    onMilestoneEvery,
    onCommit,
    onInit,
    openChapter,
    openBible,
  } = props;
  return (
    <div className="panel">
      <div className="metric-row">
        <div className="metric hero">
          <div className="metric-label">当前章节</div>
          <div className="metric-value small">{state.currentChapter || "—"}</div>
          <div className="metric-hint">
            {state.worldDate && `世界日期 ${state.worldDate} · `}
            {state.pov && `视角 ${state.pov}`}
          </div>
        </div>
        <div className="metric">
          <div className="metric-label">待回收伏笔</div>
          <div className="metric-value">{state.foreshadowingOpen}</div>
          <div className="metric-hint">来自 bible/foreshadowing.md</div>
        </div>
        <div className="metric">
          <div className="metric-label">总字数</div>
          <div className="metric-value">{fmtChars(totalChars)}</div>
          <div className="metric-hint">所有章节累加</div>
        </div>
      </div>

      <div className="git-snapshot">
        <div className="git-snapshot-head">
          <span className="git-snapshot-title">💾 Git 快照</span>
          <span className="muted small">当前 {chapterCount} 章</span>
        </div>
        <div className="git-snapshot-body">
          <label className="ctl">
            <span className="muted small">每</span>
            <select
              value={milestoneEvery}
              onChange={(e) => onMilestoneEvery(Number(e.target.value))}
            >
              <option value={0}>关闭自动提交</option>
              <option value={1}>1 章</option>
              <option value={3}>3 章</option>
              <option value={5}>5 章</option>
              <option value={10}>10 章</option>
            </select>
            <span className="muted small">自动提交一次</span>
          </label>
          <div className="spacer" />
          <button className="btn" onClick={onInit}>初始化 Git</button>
          <button className="btn primary" onClick={onCommit}>立即提交</button>
        </div>
      </div>

      {state.nextHook && (
        <div className="hook">
          <div className="hook-label">下一钩</div>
          <p>{state.nextHook}</p>
        </div>
      )}

      <h2 className="section-title">近期章节</h2>
      <ul className="recent-list">
        {recent.length === 0 && <li className="muted">还没有章节</li>}
        {recent.map((c) => (
          <li key={c.file}>
            <button className="recent-row" onClick={() => openChapter(c)}>
              <span className="recent-row-title">{c.title}</span>
              <span className="muted small">{fmtChars(c.chars)} · {fmtDate(c.modifiedMs)}</span>
            </button>
          </li>
        ))}
      </ul>

      <h2 className="section-title">圣经概览</h2>
      <div className="bible-grid">
        {bible.files.map((f) => (
          <button key={f.path} className="card" onClick={() => openBible(f.name, f.name)}>
            <div className="card-name">{f.name}</div>
            <div className="card-meta muted small">圣经文件</div>
          </button>
        ))}
        {bible.characters.map((c) => (
          <button
            key={c.path}
            className="card"
            onClick={() =>
              openBible(`characters/${c.name}`, `角色 · ${c.name}`)
            }
          >
            <div className="card-name">👤 {c.name}</div>
            <div className="card-meta muted small">角色档案</div>
          </button>
        ))}
      </div>
    </div>
  );
}

function ChaptersPanel(props: {
  chapters: Chapter[];
  onOpen: (c: Chapter) => void;
}) {
  if (props.chapters.length === 0) {
    return (
      <div className="panel">
        <p className="muted center">
          chapters/ 目录里还没有文件 —— 在 DSH Web GUI 里写第一章，或手动放进 chapters/ch01.md
        </p>
      </div>
    );
  }
  return (
    <div className="panel">
      <h2 className="section-title">所有章节（{props.chapters.length}）</h2>
      <div className="chapters">
        {props.chapters.map((c) => (
          <button key={c.path} className="chapter-row" onClick={() => props.onOpen(c)}>
            <div className="chapter-row-main">
              <div className="chapter-row-title">{c.title}</div>
              <div className="muted small">{c.file}</div>
            </div>
            <div className="chapter-row-meta">
              <span className="badge">{fmtChars(c.chars)} 字</span>
              <span className="muted small">{fmtDate(c.modifiedMs)}</span>
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}

function BiblePanel(props: {
  meta: BibleMeta;
  selected: BibleSelection | null;
  content: string;
  error: string | null;
  onSelect: (sel: BibleSelection) => void;
}) {
  const { meta, selected, content, error, onSelect } = props;
  return (
    <div className="panel">
      <div className="bible-layout">
        <div className="bible-list">
          <h2 className="section-title">圣经文件</h2>
          <ul>
            {meta.files.map((f) => (
              <li key={f.path}>
                <button
                  className={`list-item ${selected?.file === f.name ? "active" : ""}`}
                  onClick={() => onSelect({ file: f.name, display: f.name })}
                >
                  📄 {f.name}
                </button>
              </li>
            ))}
          </ul>
          {meta.characters.length > 0 && (
            <>
              <h2 className="section-title">角色</h2>
              <ul>
                {meta.characters.map((c) => (
                  <li key={c.path}>
                    <button
                      className={`list-item ${selected?.file === `characters/${c.name}` ? "active" : ""}`}
                      onClick={() =>
                        onSelect({ file: `characters/${c.name}`, display: c.name })
                      }
                    >
                      👤 {c.name}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
        <div className="bible-content">
          {selected ? (
            <>
              <h2 className="section-title">{selected.display}</h2>
              {error && <div className="error-banner">{error}</div>}
              <pre className="md">{content || "(空)"}</pre>
            </>
          ) : (
            <p className="muted center">选左侧文件查看</p>
          )}
        </div>
      </div>
    </div>
  );
}

function EditorPanel(props: {
  chapter: Chapter | null;
  draft: string;
  saved: boolean;
  saving: boolean;
  externallyModified: boolean;
  onReloadDisk: () => void;
  onChange: (v: string) => void;
  onSave: () => void;
  readMode: boolean;
  onToggleReadMode: () => void;
  immersed: boolean;
  onToggleImmerse: () => void;
  fontSize: number;
  onFontSize: (n: number) => void;
  lineHeight: number;
  onLineHeight: (n: number) => void;
  targetWords: number;
}) {
  if (!props.chapter) {
    return (
      <div className="panel">
        <p className="muted center">从章节看板选一章开始编辑</p>
      </div>
    );
  }
  const wordCount = props.draft.replace(/\s/g, "").length;
  const goal = props.targetWords;
  const reached = goal > 0 && wordCount >= goal;
  const ratio = goal > 0 ? Math.min(wordCount / goal, 1) : 0;
  const remain = goal > 0 ? Math.max(goal - wordCount, 0) : 0;

  return (
    <div className="panel editor">
      <div className="editor-toolbar">
        <div>
          <div className="editor-title">{props.chapter.title}</div>
          <div className="muted small">
            {props.chapter.file} · {fmtChars(wordCount)} 字
            {goal > 0 && ` / 目标 ${fmtChars(goal)} 字`}
          </div>
        </div>
        <div className="spacer" />
        <label className="ctl">
          <span className="muted small">字号</span>
          <select
            value={props.fontSize}
            onChange={(e) => props.onFontSize(Number(e.target.value))}
          >
            <option value={14}>小 14</option>
            <option value={16}>中 16</option>
            <option value={18}>大 18</option>
            <option value={20}>加大 20</option>
            <option value={22}>特大 22</option>
          </select>
        </label>
        <label className="ctl">
          <span className="muted small">行距</span>
          <select
            value={props.lineHeight}
            onChange={(e) => props.onLineHeight(Number(e.target.value))}
          >
            <option value={1.6}>紧凑</option>
            <option value={1.85}>标准</option>
            <option value={2.2}>宽松</option>
            <option value={2.6}>极宽</option>
          </select>
        </label>
        <button
          className={`btn ${props.readMode ? "primary" : ""}`}
          onClick={props.onToggleReadMode}
          title="阅读模式：把 Markdown 渲染成排版正文"
        >
          {props.readMode ? "✏️ 编辑" : "📖 阅读"}
        </button>
        <button
          className="btn"
          onClick={props.onToggleImmerse}
          title={props.immersed ? "退出全屏沉浸" : "全屏沉浸：隐藏界面、专注本章"}
        >
          {props.immersed ? "⤢ 退出沉浸" : "⛶ 沉浸"}
        </button>
        <span className={`badge ${props.saved ? "ok" : "warn"}`}>
          {props.saving ? "保存中…" : props.saved ? "已保存" : "有未保存的修改"}
        </span>
        <button
          className="btn primary"
          onClick={props.onSave}
          disabled={props.saving || props.saved}
        >
          保存
        </button>
      </div>

      {props.externallyModified && (
        <div className="external-banner">
          <span>⚠️ 该文件已在别处被修改（DSH agent / 插件 / 其它编辑器）。</span>
          <div className="spacer" />
          <button className="btn" onClick={props.onReloadDisk}>
            📥 重载磁盘版本
          </button>
        </div>
      )}

      {goal > 0 && (
        <WordGoalBanner wordCount={wordCount} goal={goal} reached={reached} ratio={ratio} remain={remain} />
      )}

      {props.readMode ? (
        <div
          className="md-view"
          style={{ fontSize: props.fontSize, lineHeight: props.lineHeight }}
        >
          <MarkdownView text={props.draft} />
        </div>
      ) : (
        <textarea
          className="md-editor"
          style={{ fontSize: props.fontSize, lineHeight: props.lineHeight }}
          value={props.draft}
          onChange={(e) => props.onChange(e.target.value)}
          spellCheck={false}
        />
      )}
    </div>
  );
}

function WordGoalBanner(props: {
  wordCount: number;
  goal: number;
  reached: boolean;
  ratio: number;
  remain: number;
}) {
  if (props.reached) {
    return (
      <div className="goal-banner reached">
        <span className="goal-icon">🎉</span>
        <div className="goal-text">
          <div className="goal-title">已达成章节字数目标</div>
          <div className="muted small">
            {fmtChars(props.wordCount)} / {fmtChars(props.goal)} 字 —— 这一章可以收尾了
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="goal-banner">
      <div className="goal-head">
        <span className="goal-icon">🎯</span>
        <span className="goal-title">字数目标</span>
        <span className="muted small goal-remain">还差 {fmtChars(props.remain)} 字</span>
      </div>
      <div className="goal-track">
        <div className="goal-fill" style={{ width: `${(props.ratio * 100).toFixed(1)}%` }} />
      </div>
      <div className="muted small goal-foot">
        {fmtChars(props.wordCount)} / {fmtChars(props.goal)} 字 · {Math.round(props.ratio * 100)}%
      </div>
    </div>
  );
}
