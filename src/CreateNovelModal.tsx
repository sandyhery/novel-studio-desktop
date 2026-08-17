import { useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import {
  deriveProjectName,
  deriveSingleChapterWords,
  GENRES,
  POV_MODES,
  presetEras,
  TONES,
  type Genre,
  type NovelBrief,
  type PovMode,
  type Tone,
} from "./types";

type Step = 0 | 1 | 2 | 3;

interface CreateNovelModalProps {
  onClose: () => void;
  onCreated: (newProjectRoot: string) => void;
  initialParent?: string | null;
}

const STEP_LABELS = [
  "基础",
  "题材 / 视角",
  "规模",
  "开局要素",
];

export default function CreateNovelModal({
  onClose,
  onCreated,
  initialParent = null,
}: CreateNovelModalProps) {
  const [step, setStep] = useState<Step>(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [brief, setBrief] = useState<NovelBrief>({
    parent: initialParent ?? "",
    name: "",
    title: "",
    povMode: "第三人称",
    povCharacter: "",
    genre: "玄幻",
    tone: "严肃",
    era: "架空",
    targetWordsWan: 100,
    volumes: 3,
    chaptersPerVolume: 30,
    coreConflict: "",
    heroSituation: "",
    heroDesire: "",
    openingHook: "",
  });

  const set = <K extends keyof NovelBrief>(k: K, v: NovelBrief[K]) =>
    setBrief((b) => ({ ...b, [k]: v }));

  const chapterWord = useMemo(() => deriveSingleChapterWords(brief), [brief]);
  const totalChapters = brief.volumes * brief.chaptersPerVolume;

  const pickParent = async () => {
    setError(null);
    try {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) set("parent", picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const goNext = () => {
    setError(null);
    if (step === 0) {
      if (!brief.parent) return setError("请先选父目录");
      if (!brief.title.trim()) return setError("标题不能为空");
      if (!brief.name.trim()) {
        set("name", deriveProjectName(brief.title));
      }
    }
    if (step === 1) {
      if (brief.povMode !== "多重视角" && !brief.povCharacter.trim()) {
        return setError("非多重视角时需要填 POV 主角");
      }
    }
    if (step === 2) {
      if (brief.targetWordsWan <= 0) return setError("字数目标必须 > 0");
      if (brief.volumes <= 0) return setError("卷数必须 > 0");
      if (brief.chaptersPerVolume <= 0) return setError("每卷章数必须 > 0");
    }
    if (step === 3) {
      if (!brief.coreConflict.trim()) return setError("核心矛盾不能为空");
      if (!brief.heroSituation.trim()) return setError("主角处境不能为空");
      if (!brief.heroDesire.trim()) return setError("主角欲望不能为空");
      if (!brief.openingHook.trim()) return setError("第一幕核心冲突不能为空");
    }
    setStep((s) => (Math.min(3, s + 1) as Step));
  };
  const goPrev = () => {
    setError(null);
    setStep((s) => (Math.max(0, s - 1) as Step));
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const newRoot = await invoke<string>("create_novel", { args: brief });
      onCreated(newRoot);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <div>
            <h2>新建小说项目</h2>
            <p className="muted small">填完 4 步会调用 agt novel-init 自动创建骨架</p>
          </div>
          <button className="btn ghost" onClick={onClose} aria-label="关闭">✕</button>
        </header>

        <ol className="stepper">
          {STEP_LABELS.map((label, i) => (
            <li
              key={label}
              className={`${i === step ? "active" : ""} ${i < step ? "done" : ""}`}
              onClick={() => i <= step && setStep(i as Step)}
            >
              <span className="step-dot">{i + 1}</span>
              <span className="step-label">{label}</span>
            </li>
          ))}
        </ol>

        {error && <div className="error-banner">{error}</div>}

        <section className="modal-body">
          {step === 0 && (
            <div className="form-grid">
              <label className="label">父目录</label>
              <div className="form-row">
                <input
                  className="search"
                  placeholder="选个目录，新项目会创建在它下面"
                  value={brief.parent}
                  onChange={(e) => set("parent", e.target.value)}
                />
                <button className="btn" onClick={pickParent}>选…</button>
              </div>
              <p className="form-hint">
                推荐 <code>~/Documents/novels</code> 或 <code>~/novels</code>。
                不要选 dsh web 的 home 或大项目根目录。
              </p>

              <label className="label">标题</label>
              <input
                className="search"
                placeholder="例：山海猎人 / 凉城往事 / 时之钥"
                value={brief.title}
                onChange={(e) => {
                  set("title", e.target.value);
                  if (!brief.name) set("name", deriveProjectName(e.target.value));
                }}
              />

              <label className="label">目录名</label>
              <input
                className="search"
                placeholder="例：shanhai-lieren"
                value={brief.name}
                onChange={(e) => set("name", e.target.value)}
              />
              <p className="form-hint">
                项目会以这个名字创建为子目录。留空会自动从标题派生（去标点、连字符替空格）。
              </p>
            </div>
          )}

          {step === 1 && (
            <div className="form-grid">
              <label className="label">类型</label>
              <div className="chip-row">
                {GENRES.map((g) => (
                  <button
                    key={g}
                    className={`chip ${brief.genre === g ? "active" : ""}`}
                    onClick={() => set("genre", g as Genre)}
                  >
                    {g}
                  </button>
                ))}
              </div>

              <label className="label">基调</label>
              <div className="chip-row">
                {TONES.map((t) => (
                  <button
                    key={t}
                    className={`chip ${brief.tone === t ? "active" : ""}`}
                    onClick={() => set("tone", t as Tone)}
                  >
                    {t}
                  </button>
                ))}
              </div>

              <label className="label">时代背景</label>
              <input
                className="search"
                list="preset-eras"
                placeholder="例：架空 / 现代都市 / 唐贞观 / 未来 2187"
                value={brief.era}
                onChange={(e) => set("era", e.target.value)}
              />
              <datalist id="preset-eras">
                {presetEras().map((e) => (
                  <option key={e} value={e} />
                ))}
              </datalist>
              <p className="form-hint">下拉选预设，或自己填任意时间（公元 1887 / 银河纪 47 年）。</p>

              <label className="label">视角</label>
              <div className="chip-row">
                {POV_MODES.map((m) => (
                  <button
                    key={m}
                    className={`chip ${brief.povMode === m ? "active" : ""}`}
                    onClick={() => set("povMode", m as PovMode)}
                  >
                    {m}
                  </button>
                ))}
              </div>

              {brief.povMode !== "多重视角" && (
                <>
                  <label className="label">POV 主角名</label>
                  <input
                    className="search"
                    placeholder="例：林楚 / 凉生 / 萧承"
                    value={brief.povCharacter}
                    onChange={(e) => set("povCharacter", e.target.value)}
                  />
                  <p className="form-hint">
                    第一章的 POV，未来建立角色档案 <code>bible/characters/&lt;名&gt;.md</code> 时用这个。
                  </p>
                </>
              )}
            </div>
          )}

          {step === 2 && (
            <div className="form-grid">
              <label className="label">字数目标（万字）</label>
              <input
                className="search"
                type="number"
                min={1}
                max={5000}
                value={brief.targetWordsWan}
                onChange={(e) =>
                  set("targetWordsWan", Math.max(1, Number(e.target.value) || 0))
                }
              />
              <p className="form-hint">
                网文常见：玄幻长篇 200–500 万；中篇 30–80 万；短篇 5–15 万。
              </p>

              <label className="label">卷数</label>
              <input
                className="search"
                type="number"
                min={1}
                max={50}
                value={brief.volumes}
                onChange={(e) =>
                  set("volumes", Math.max(1, Number(e.target.value) || 0))
                }
              />

              <label className="label">每卷章数</label>
              <input
                className="search"
                type="number"
                min={1}
                max={500}
                value={brief.chaptersPerVolume}
                onChange={(e) =>
                  set("chaptersPerVolume", Math.max(1, Number(e.target.value) || 0))
                }
              />

              <div className="metric hero">
                <div className="metric-label">预估</div>
                <div className="metric-value small">
                  共 {totalChapters} 章 · 单章约 {chapterWord.toLocaleString()} 字
                </div>
                <div className="metric-hint muted small">
                  网文单章 2000–3500 字为常见节奏；超长章（8000+）可降低成本。
                </div>
              </div>
            </div>
          )}

          {step === 3 && (
            <div className="form-grid">
              <label className="label">核心矛盾</label>
              <textarea
                className="md-editor small"
                rows={2}
                placeholder="一句话。例如：凡人闯入禁区，发现自己的影子在指挥军队。"
                value={brief.coreConflict}
                onChange={(e) => set("coreConflict", e.target.value)}
              />
              <p className="form-hint">
                全书的<strong>主轴冲突</strong>：决定每一卷都在围绕什么升级。
              </p>

              <label className="label">主角处境</label>
              <textarea
                className="md-editor small"
                rows={2}
                placeholder="第一章开篇主角在干嘛？例：北漂第三年，刚被房东赶出来，手机欠费。"
                value={brief.heroSituation}
                onChange={(e) => set("heroSituation", e.target.value)}
              />

              <label className="label">主角欲望</label>
              <textarea
                className="md-editor small"
                rows={2}
                placeholder="主角想要什么？例：回老家、救出失踪的妹妹、在三年内还清欠款。"
                value={brief.heroDesire}
                onChange={(e) => set("heroDesire", e.target.value)}
              />

              <label className="label">第一幕核心冲突</label>
              <textarea
                className="md-editor small"
                rows={2}
                placeholder="在 ch1 内必须立刻发生的不可逆事件。例：捡到一只会说话的乌鸦，发现自己在倒计时 30 天。"
                value={brief.openingHook}
                onChange={(e) => set("openingHook", e.target.value)}
              />
              <p className="form-hint">
                这段会<strong>直接落到</strong> <code>state.yml → next_hook</code> 与
                <code> bible/foreshadowing.md → F001</code>，是 DSH 模型写 ch1 的第一锚点。
              </p>
            </div>
          )}
        </section>

        <footer className="modal-foot">
          <button className="btn" onClick={onClose} disabled={busy}>取消</button>
          <div className="spacer" />
          {step > 0 && (
            <button className="btn" onClick={goPrev} disabled={busy}>上一步</button>
          )}
          {step < 3 && (
            <button className="btn primary" onClick={goNext} disabled={busy}>
              下一步 →
            </button>
          )}
          {step === 3 && (
            <button className="btn primary" onClick={submit} disabled={busy}>
              {busy ? "创建中…" : "✓ 创建项目"}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
