import type { ChoicePoint } from "./types";
import { weightLabel, weightTone } from "./types";

export interface ChoiceRequest {
  prompt: string;
  options: Array<{ id: string; label: string; previewHint: string }>;
}

/**
 * 通用抉择模态框：既用于"已持久化抉择点"的查看/变更（cp: ChoicePoint），
 * 也用于"AI 写作中实时请求抉择"（req: ChoiceRequest）。
 * 二选一传入。
 */
export default function ChoiceModal(props: {
  cp?: ChoicePoint | null;
  req?: ChoiceRequest | null;
  title?: string;
  subtitle?: string;
  pickedOption: string;
  note: string;
  onPick: (id: string) => void;
  onNoteChange: (v: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
}) {
  const {
    cp = null,
    req = null,
    title,
    subtitle,
    pickedOption,
    note,
    onPick,
    onNoteChange,
    onConfirm,
    onCancel,
    confirmLabel,
  } = props;

  const promptText = cp?.prompt ?? req?.prompt ?? "";
  const options = cp?.options ?? req?.options ?? [];
  const weight = cp?.weight ?? "major";

  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal choice-modal" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <div>
            <h2>{title ?? "做决定"}</h2>
            {cp && (
              <span className={`badge ${weightTone(weight)}`}>{weightLabel(weight)}</span>
            )}
            {cp && <span className="muted small" style={{ marginLeft: 8 }}>{cp.id}</span>}
            {subtitle && <span className="muted small" style={{ marginLeft: 8 }}>{subtitle}</span>}
          </div>
          <button className="btn ghost" onClick={onCancel} aria-label="关闭">✕</button>
        </header>

        <section className="modal-body">
          <p className="choice-prompt">{promptText}</p>
          <ul className="options-list large">
            {options.map((o) => {
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
            {confirmLabel ?? (pickedOption === "ai" ? "让 AI 决定（10 秒内可覆盖）" : "✓ 锁定这个决定")}
          </button>
        </footer>
      </div>
    </div>
  );
}
