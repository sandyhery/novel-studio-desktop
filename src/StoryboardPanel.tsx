import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  type ChoicePoint,
  type ChoicePointsView,
  weightLabel,
  weightTone,
} from "./types";

interface StoryboardPanelProps {
  root: string;
}

export default function StoryboardPanel({ root }: StoryboardPanelProps) {
  const [view, setView] = useState<ChoicePointsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [seeding, setSeeding] = useState(false);
  // 当前正在做决定的抉择点（弹窗态）
  const [deciding, setDeciding] = useState<ChoicePoint | null>(null);
  const [pickedOption, setPickedOption] = useState<string>("");
  const [decisionNote, setDecisionNote] = useState<string>("");

  const reload = useCallback(async () => {
    setError(null);
    try {
      const v = await invoke<ChoicePointsView>("read_choice_points", { root });
      setView(v);
    } catch (e) {
      setError(String(e));
      setView(null);
    }
  }, [root]);

  useEffect(() => {
    reload();
  }, [reload]);

  const seedDemo = async () => {
    setSeeding(true);
    setError(null);
    try {
      const added = await invoke<number>("seed_demo_choice_points", { root });
      if (added > 0) await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setSeeding(false);
    }
  };

  const startDecide = (cp: ChoicePoint) => {
    setDeciding(cp);
    setPickedOption(cp.decided?.optionId ?? "");
    setDecisionNote(cp.decided?.note ?? "");
  };

  const cancelDecide = () => {
    setDeciding(null);
    setPickedOption("");
    setDecisionNote("");
  };

  const confirmDecide = async () => {
    if (!deciding || !pickedOption) return;
    try {
      await invoke("decide_choice_point", {
        args: {
          root,
          pointId: deciding.id,
          optionId: pickedOption,
          by: "human",
          note: decisionNote.trim() || null,
        },
      });
      cancelDecide();
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  // ---------------------------------------------------------------------
  // 渲染
  // ---------------------------------------------------------------------

  if (!view) {
    return (
      <div className="panel">
        {error && <div className="error-banner">{error}</div>}
        <p className="muted center">加载抉择点状态…</p>
      </div>
    );
  }

  if (!view.aiNovelDirExists) {
    return (
      <div className="panel">
        <div className="empty">
          <h2>还没有 .ai-novel 目录</h2>
          <p className="muted">
            这个项目尚未启用"抉择点"机制。点下面按钮塞 3 个示例抉择点，看看效果。
          </p>
          <button
            className="btn primary"
            onClick={seedDemo}
            disabled={seeding}
          >
            {seeding ? "正在创建示例…" : "✨ 创建 3 个示例抉择点"}
          </button>
          {error && <div className="error-banner" style={{ marginTop: 12 }}>{error}</div>}
        </div>
      </div>
    );
  }

  const pending = view.points.filter((p) => p.decided === null);
  const decided = view.points.filter((p) => p.decided !== null);

  return (
    <div className="panel">
      {error && <div className="error-banner">{error}</div>}

      <div className="metric-row">
        <div className="metric hero">
          <div className="metric-label">总抉择点数</div>
          <div className="metric-value">{view.points.length}</div>
          <div className="metric-hint">
            跨整本小说，每个抉择点会改变后续走向
          </div>
        </div>
        <div className="metric">
          <div className="metric-label">已做决定</div>
          <div className="metric-value small">{view.decidedCount}</div>
          <div className="metric-hint">走向已锁定</div>
        </div>
        <div className="metric">
          <div className="metric-label">待决定</div>
          <div className="metric-value small" style={{ color: "var(--warn)" }}>
            {view.pendingCount}
          </div>
          <div className="metric-hint">需要你或 AI 做出选择</div>
        </div>
      </div>

      <h2 className="section-title">待决定（{pending.length}）</h2>
      {pending.length === 0 && (
        <p className="muted center">🎉 目前没有等待你决定的抉择点。</p>
      )}
      <div className="grid choice-grid">
        {pending.map((cp) => (
          <ChoiceCard
            key={cp.id}
            cp={cp}
            onDecide={() => startDecide(cp)}
            actionLabel={cp.weight === "critical" ? "马上定" : "做决定"}
          />
        ))}
      </div>

      <h2 className="section-title">已决定（{decided.length}）</h2>
      <div className="grid choice-grid">
        {decided.map((cp) => (
          <ChoiceCard key={cp.id} cp={cp} onDecide={() => startDecide(cp)} actionLabel="变更" />
        ))}
      </div>

      {deciding && (
        <DecideModal
          cp={deciding}
          pickedOption={pickedOption}
          note={decisionNote}
          onPick={setPickedOption}
          onNoteChange={setDecisionNote}
          onConfirm={confirmDecide}
          onCancel={cancelDecide}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------
// 单个抉择点卡片
// ---------------------------------------------------------------------

function ChoiceCard(props: {
  cp: ChoicePoint;
  onDecide: () => void;
  actionLabel: string;
}) {
  const { cp, onDecide, actionLabel } = props;
  const isDecided = cp.decided !== null;
  return (
    <article className="card choice-card">
      <div className="card-head">
        <h3 className="card-name">{cp.prompt}</h3>
        <span className={`badge ${weightTone(cp.weight)}`}>{weightLabel(cp.weight)}</span>
      </div>
      <div className="card-meta">
        <span className="muted">{cp.id} · 在 {cp.afterChapter} 之后</span>
      </div>
      <ul className="options-list">
        {cp.options.map((o) => {
          const chosen = isDecided && cp.decided?.optionId === o.id;
          return (
            <li key={o.id} className={chosen ? "option chosen" : "option"}>
              <div className="option-head">
                <span className="option-label">
                  {chosen && <span className="chosen-mark">✓</span>} {o.label}
                </span>
                <span className="muted small">{o.id}</span>
              </div>
              <p className="option-hint">{o.previewHint}</p>
            </li>
          );
        })}
      </ul>
      {isDecided && cp.decided && (
        <div className="decision-strip">
          <div>
            <span className="muted small">决定者</span>
            <span>{cp.decided.by === "human" ? "你" : "AI"}</span>
          </div>
          {cp.decided.note && (
            <div className="decision-note">"{cp.decided.note}"</div>
          )}
        </div>
      )}
      <div className="card-foot">
        <button className="btn primary" onClick={onDecide}>
          {isDecided ? "✎ 改主意" : actionLabel}
        </button>
      </div>
    </article>
  );
}

// ---------------------------------------------------------------------
// 决定模态
// ---------------------------------------------------------------------

function DecideModal(props: {
  cp: ChoicePoint;
  pickedOption: string;
  note: string;
  onPick: (id: string) => void;
  onNoteChange: (v: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { cp, pickedOption, note, onPick, onNoteChange, onConfirm, onCancel } = props;
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal choice-modal" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <div>
            <h2>做决定 — {cp.id}</h2>
            <span className={`badge ${weightTone(cp.weight)}`}>{weightLabel(cp.weight)}</span>
            <span className="muted small" style={{ marginLeft: 8 }}>
              在 {cp.afterChapter} 之后
            </span>
          </div>
          <button className="btn ghost" onClick={onCancel} aria-label="关闭">✕</button>
        </header>

        <section className="modal-body">
          <p className="choice-prompt">{cp.prompt}</p>
          <ul className="options-list large">
            {cp.options.map((o) => {
              const isPicked = pickedOption === o.id;
              const isAi = o.id === "ai";
              return (
                <li
                  key={o.id}
                  className={`option pickable ${isPicked ? "chosen" : ""} ${isAi ? "ai-option" : ""}`}
                  onClick={() => onPick(o.id)}
                >
                  <div className="option-head">
                    <span className="option-label">
                      {isPicked && <span className="chosen-mark">✓</span>} {o.label}
                    </span>
                    <span className="muted small">{o.id}</span>
                  </div>
                  <p className="option-hint">{o.previewHint}</p>
                </li>
              );
            })}
          </ul>

          <h3 className="section-title" style={{ marginTop: 18 }}>
            决定后写一句（可选）
          </h3>
          <textarea
            className="md-editor small"
            rows={3}
            placeholder="例：希望萧承活着，三个月后再次成为关键角色……"
            value={note}
            onChange={(e) => onNoteChange(e.target.value)}
          />
          <p className="muted small">
            AI 后续写作时会看到你的决定 + 这条备注；
            你也可以选择「让 AI 决定」让模型基于剧情张力自选。
          </p>
        </section>

        <footer className="modal-foot">
          <button className="btn" onClick={onCancel}>取消</button>
          <div className="spacer" />
          <button
            className="btn primary"
            onClick={onConfirm}
            disabled={!pickedOption}
          >
            {pickedOption === "ai" ? "让 AI 决定（10 秒内可覆盖）" : "✓ 锁定这个决定"}
          </button>
        </footer>
      </div>
    </div>
  );
}
