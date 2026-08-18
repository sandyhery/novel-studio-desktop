import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  type AiReviseChapterResult,
  type DshModelGroup,
  type ReviewReport,
  REVIEW_CATEGORY_ORDER,
} from "./types";

interface ReviewPanelProps {
  root: string;
  sessionId?: string | null;
  onSessionId?: (sid: string | null) => void;
  onSaved?: () => void;
  dshLoggedIn?: boolean;
  onLogin?: () => void;
}

const SEVERITY_TONE: Record<string, string> = {
  critical: "danger",
  major: "warn",
  minor: "",
  info: "",
};

export default function ReviewPanel({
  root,
  sessionId,
  onSessionId,
  onSaved,
  dshLoggedIn,
  onLogin,
}: ReviewPanelProps) {
  const [report, setReport] = useState<ReviewReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revising, setRevising] = useState(false);
  const [revisedText, setRevisedText] = useState<string | null>(null);
  const [modelGroups, setModelGroups] = useState<DshModelGroup[]>([]);
  const [selectedModel, setSelectedModel] = useState("");

  // 拉取 DSH 可用模型目录（仅登录后）
  useEffect(() => {
    if (!dshLoggedIn) return;
    let cancelled = false;
    invoke<DshModelGroup[]>("dsh_list_models")
      .then((groups) => {
        if (cancelled) return;
        setModelGroups(groups);
      })
      .catch(() => {
        if (!cancelled) setModelGroups([]);
      });
    return () => {
      cancelled = true;
    };
  }, [dshLoggedIn]);

  // 从 "provider||model" 拆出本次审核要用的模型（空则用 DSH 默认）
  const modelSelection = useCallback(() => {
    if (!selectedModel) return { modelProvider: null as string | null, modelId: null as string | null };
    const [provider, model] = selectedModel.split("||");
    return { modelProvider: provider || null, modelId: model || null };
  }, [selectedModel]);

  const review = useCallback(async (target?: string) => {
    setBusy(true);
    setError(null);
    setRevisedText(null);
    const { modelProvider, modelId } = modelSelection();
    try {
      const r = await invoke<ReviewReport>("ai_review_chapter", {
        args: {
          root,
          chapterFile: target || null,
          sessionId: sessionId ?? null,
          modelProvider,
          modelId,
          timeoutSecs: 300,
        },
      });
      setReport(r);
      if (r.sessionId) onSessionId?.(r.sessionId);
      if (r.error) setError(r.error);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [root, sessionId, onSessionId, modelSelection]);

  const revise = useCallback(async () => {
    if (!report) return;
    setRevising(true);
    setError(null);
    setRevisedText(null);
    try {
      const res = await invoke<AiReviseChapterResult>("ai_revise_chapter", {
        args: {
          root,
          chapterFile: report.chapterFile,
          reportJson: JSON.stringify(report),
          sessionId: sessionId ?? null,
          timeoutSecs: 300,
        },
      });
      if (res.sessionId) onSessionId?.(res.sessionId);
      if (res.error) {
        setError(res.error);
      } else {
        setRevisedText(res.revisedText);
        onSaved?.();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setRevising(false);
    }
  }, [report, root, sessionId, onSessionId, onSaved]);

  const countIssues = useCallback((r: ReviewReport) => {
    return Object.values(r.categories).reduce((sum, c) => sum + c.issues.length, 0);
  }, []);

  return (
    <div className="panel">
      {dshLoggedIn === false && (
        <div className="external-banner">
          <span>⚠️ 未登录 DSH Web —— AI 审核需要先登录。</span>
          <div className="spacer" />
          <button className="btn primary" onClick={onLogin}>🔐 登录 DSH</button>
        </div>
      )}

      <div className="toolbar" style={{ marginTop: 0 }}>
        <h2 className="section-title" style={{ margin: 0 }}>🛡️ AI 审核员</h2>
        <div className="spacer" />
        {dshLoggedIn && (
          <>
            <span className="muted small">审核模型</span>
            <select
              className="search"
              style={{ maxWidth: 300 }}
              value={selectedModel}
              onChange={(e) => setSelectedModel(e.target.value)}
              title="本次审核使用的模型（用完自动恢复默认，不改全局）"
            >
              <option value="">跟随 DSH 默认</option>
              {modelGroups.map((g) => (
                <optgroup key={g.id} label={g.name}>
                  {g.models.map((m) => (
                    <option key={`${g.id}||${m.id}`} value={`${g.id}||${m.id}`}>
                      {m.name}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
          </>
        )}
        <button className="btn primary" onClick={() => review()} disabled={busy}>
          {busy ? "审核中…" : "审核最近一章"}
        </button>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {report && report.ok && (
        <>
          <div className="review-summary">
            <div className="review-verdict">
              <span className={`badge ${report.verdict === "pass" ? "ok" : "warn"}`}>
                {report.verdict === "pass" ? "✓ 通过" : "⚠ 建议修订"}
              </span>
              <span className="muted small">章节：{report.chapterFile} · {countIssues(report)} 条问题</span>
            </div>
            <p className="review-summary-text">{report.summary || "（无总结）"}</p>
          </div>

          {REVIEW_CATEGORY_ORDER.map((key) => {
            const cat = report.categories[key];
            if (!cat || cat.issues.length === 0) return null;
            return (
              <div key={key} className="review-category">
                <h3 className="review-cat-head">
                  <span className="badge">{key}</span> {cat.label}
                  <span className="muted small">（{cat.issues.length}）</span>
                </h3>
                <ul className="review-issues">
                  {cat.issues.map((issue, i) => (
                    <li key={i} className="review-issue">
                      <div className="review-issue-head">
                        <span className={`badge ${SEVERITY_TONE[issue.severity] ?? ""}`}>
                          {issue.severity}
                        </span>
                        {issue.location && (
                          <span className="muted small">{issue.location}</span>
                        )}
                      </div>
                      <p className="review-issue-text">{issue.issue}</p>
                      {issue.suggestion && (
                        <p className="review-suggestion">💡 {issue.suggestion}</p>
                      )}
                    </li>
                  ))}
                </ul>
              </div>
            );
          })}

          {countIssues(report) === 0 && (
            <p className="muted center">🎉 没有发现问题，这章可以接受。</p>
          )}

          <div className="toolbar">
            <div className="spacer" />
            <button className="btn primary" onClick={revise} disabled={revising}>
              {revising ? "AI 修订中…" : "✍️ AI 修订（生成 v2）"}
            </button>
          </div>
        </>
      )}

      {revisedText && (
        <div className="result-box">
          <div className="result-head">
            <span className="badge ok">已修订并保存 v2</span>
            <button className="btn" onClick={onSaved}>刷新看板</button>
          </div>
          <p className="muted small">原版备份在 <code>{report?.chapterFile}.bak</code></p>
          <pre className="md">{revisedText.slice(0, 1500)}{revisedText.length > 1500 ? "…（完整内容已存盘）" : ""}</pre>
        </div>
      )}

      {!report && !busy && !error && (
        <p className="muted center">
          点「审核最近一章」让 AI 以连续性守门员身份检查
          严重冲突 / 性格漂移 / 信息差 / 伏笔 / 节奏五类问题。
        </p>
      )}
    </div>
  );
}
