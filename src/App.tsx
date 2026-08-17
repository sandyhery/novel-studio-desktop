import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { BibleMeta, Chapter, NovelState, NovelSummary, Probe } from "./types";
import { fmtChars, fmtDate } from "./types";
import CreateNovelModal from "./CreateNovelModal";
import StoryboardPanel from "./StoryboardPanel";
import AiWritePanel from "./AiWritePanel";

type Panel = "overview" | "chapters" | "bible" | "editor" | "storyboard" | "aiwrite";

interface BibleSelection {
  /** "timeline" / "foreshadowing" / "characters/林楚" 这种带不带路径的 key */
  file: string;
  display: string;
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
  const [showCreate, setShowCreate] = useState(false);
  const [dshSessionId, setDshSessionId] = useState<string | null>(null);
  const [probe, setProbe] = useState<Probe | null>(null);
  const [probeFor, setProbeFor] = useState<string | null>(null);
  const [briefInit, setBriefInit] = useState<{ parent?: string | null; name?: string } | null>(null);

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
    async (rootPath: string) => {
      setBusy(true);
      setError(null);
      setSummary(null);
      setProbe(null);
      setProbeFor(rootPath);
      try {
        const s = await invoke<NovelSummary>("read_summary", { root: rootPath });
        if (!s.ok) {
          setError("read_summary 返回 ok=false");
          await runProbe(rootPath);
          return;
        }
        setSummary(s);
        setRoot(s.root);
        setSelectedBible(null);
        setBibleContent("");
      } catch (e) {
        setError(String(e));
        await runProbe(rootPath);
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
    try {
      const text = await invoke<string>("read_chapter", { root, file: chap.file });
      setChapterDraft(text);
    } catch (e) {
      setError(String(e));
    }
  }, [root]);

  const saveChapter = useCallback(async () => {
    if (!root || !editingChapter) return;
    setSaving(true);
    try {
      await invoke("write_chapter", {
        args: {
          root,
          file: editingChapter.file,
          content: chapterDraft,
        },
      });
      setChapterSaved(true);
      await loadSummary(root);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, [root, editingChapter, chapterDraft, loadSummary]);

  const totalChars = useMemo(
    () => summary?.chapters.reduce((sum, c) => sum + c.chars, 0) ?? 0,
    [summary],
  );

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
            />
          )}

          {summary && panel === "editor" && (
            <EditorPanel
              chapter={editingChapter}
              draft={chapterDraft}
              saved={chapterSaved}
              saving={saving}
              onChange={(v) => {
                setChapterDraft(v);
                setChapterSaved(false);
              }}
              onSave={saveChapter}
            />
          )}
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
  openChapter: (c: Chapter) => void;
  openBible: (file: string, display: string) => void;
}) {
  const { state, bible, recent, totalChars, openChapter, openBible } = props;
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
  onChange: (v: string) => void;
  onSave: () => void;
}) {
  if (!props.chapter) {
    return (
      <div className="panel">
        <p className="muted center">从章节看板选一章开始编辑</p>
      </div>
    );
  }
  const wordCount = props.draft.replace(/\s/g, "").length;
  return (
    <div className="panel editor">
      <div className="editor-toolbar">
        <div>
          <div className="editor-title">{props.chapter.title}</div>
          <div className="muted small">{props.chapter.file} · {fmtChars(wordCount)} 字</div>
        </div>
        <div className="spacer" />
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
      <textarea
        className="md-editor"
        value={props.draft}
        onChange={(e) => props.onChange(e.target.value)}
        spellCheck={false}
      />
    </div>
  );
}
