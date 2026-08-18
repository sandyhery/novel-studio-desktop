import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AiFullPipelineResult, AiReconcileBibleResult, AiWriteChapterResult, DshModelGroup } from "./types";
import ChoiceModal, { type ChoiceRequest } from "./ChoiceModal";

interface AiWritePanelProps {
  root: string;
  sessionId?: string | null;
  onSessionId?: (sid: string | null) => void;
  onSaved?: () => void; // 章节保存后刷新
  dshLoggedIn?: boolean;
  onLogin?: () => void;
}

type Phase = "idle" | "running" | "choice" | "done" | "error";

export default function AiWritePanel({
  root,
  sessionId,
  onSessionId,
  onSaved,
  dshLoggedIn,
  onLogin,
}: AiWritePanelProps) {
  const [instruction, setInstruction] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [progress, setProgress] = useState("");
  const [result, setResult] = useState<AiWriteChapterResult | null>(null);
  const [choiceReq, setChoiceReq] = useState<ChoiceRequest | null>(null);
  const [pickedOption, setPickedOption] = useState("");
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [reconciling, setReconciling] = useState(false);
  const [reconcileOutcome, setReconcileOutcome] = useState<string | null>(null);
  const [pipelineRunning, setPipelineRunning] = useState(false);
  const [pipelineResult, setPipelineResult] = useState<AiFullPipelineResult | null>(null);
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

  // 从 "provider||model" 拆出本次写作要用的模型（空则用 DSH 默认）
  const modelSelection = useCallback(() => {
    if (!selectedModel) return { modelProvider: null as string | null, modelId: null as string | null };
    const [provider, model] = selectedModel.split("||");
    return { modelProvider: provider || null, modelId: model || null };
  }, [selectedModel]);

  const startWrite = useCallback(async () => {
    setError(null);
    setResult(null);
    setPhase("running");
    setProgress("正在创建 DSH 会话并让 AI 写作…（长文可能需 1-3 分钟）");
    const { modelProvider, modelId } = modelSelection();
    try {
      const res = await invoke<AiWriteChapterResult>("ai_write_chapter", {
        args: {
          root,
          instruction: instruction.trim() || null,
          sessionId: sessionId ?? null,
          modelProvider,
          modelId,
          timeoutSecs: 300,
        },
      });
      if (!res.ok) {
        setError(res.error ?? "AI 写作失败");
        setPhase("error");
        return;
      }
      if (res.sessionId) onSessionId?.(res.sessionId);
      if (res.choiceRequest) {
        setChoiceReq(res.choiceRequest);
        setPickedOption("");
        setNote("");
        setPhase("choice");
        setProgress("AI 遇到了一个关键抉择，需要你决定方向。");
      } else {
        setResult(res);
        setPhase("done");
        setProgress(res.savedTo ? `已保存到 ${res.savedTo}` : "AI 已完成（未保存文件）");
        if (res.savedTo) onSaved?.();
      }
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }, [root, instruction, sessionId, onSessionId, onSaved, modelSelection]);

  const confirmChoice = useCallback(async () => {
    if (!choiceReq || !pickedOption) return;
    setPhase("running");
    setChoiceReq(null);
    // 把决定作为续写指令
    const chosen = choiceReq.options.find((o) => o.id === pickedOption);
    const continueText =
      pickedOption === "ai"
        ? "之前我让你在抉择点停下来。请你自己评估三种走向的剧情张力，选择一个最有戏剧性的继续写下去。"
        : `之前我让你在抉择点停下来。人类已决定选择：「${chosen?.label ?? pickedOption}」。${note ? `备注：${note}` : ""} 请基于这个决定继续写下去（不要重写已写内容）。`;
    setInstruction(continueText);
    setProgress("已收到你的决定，AI 继续写作…");
    const { modelProvider, modelId } = modelSelection();
    try {
      const res = await invoke<AiWriteChapterResult>("ai_write_chapter", {
        args: {
          root,
          instruction: continueText,
          sessionId: sessionId ?? null,
          modelProvider,
          modelId,
          timeoutSecs: 300,
        },
      });
      if (res.sessionId) onSessionId?.(res.sessionId);
      if (res.choiceRequest) {
        setChoiceReq(res.choiceRequest);
        setPickedOption("");
        setNote("");
        setPhase("choice");
      } else {
        setResult(res);
        setPhase("done");
        setProgress(res.savedTo ? `已保存到 ${res.savedTo}` : "AI 已完成");
        if (res.savedTo) onSaved?.();
      }
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }, [choiceReq, pickedOption, note, root, sessionId, onSessionId, onSaved, modelSelection]);

  const cancelChoice = useCallback(() => {
    setChoiceReq(null);
    setPickedOption("");
    setPhase("idle");
  }, []);

  const reconcile = useCallback(async () => {
    if (!result?.savedTo) return;
    setReconciling(true);
    setReconcileOutcome(null);
    try {
      const res = await invoke<AiReconcileBibleResult>("ai_reconcile_bible", {
        args: {
          root,
          chapterFile: result.savedTo,
          sessionId: sessionId ?? null,
          timeoutSecs: 180,
        },
      });
      if (res.sessionId) onSessionId?.(res.sessionId);
      setReconcileOutcome(res.ok
        ? (res.error ?? "已同步到圣经（timeline / foreshadowing / 角色）")
        : `同步失败：${res.error ?? ""}`);
      setProgress(res.ok ? "✅ 圣经已更新（收尾三件事完成）" : "⚠️ 圣经同步出问题");
    } catch (e) {
      setReconcileOutcome(`同步失败：${String(e)}`);
    } finally {
      setReconciling(false);
    }
  }, [result, root, sessionId, onSessionId]);

  const runPipeline = useCallback(async () => {
    setError(null);
    setResult(null);
    setPipelineResult(null);
    setPipelineRunning(true);
    setProgress("🚀 一键流水线：写 → 审 → 改 → 收尾…（全程约 3-6 分钟）");
    try {
      const res = await invoke<AiFullPipelineResult>("ai_full_pipeline", {
        args: {
          root,
          instruction: instruction.trim() || null,
          autoRevise: true,
          autoReconcile: true,
          sessionId: sessionId ?? null,
          timeoutSecs: 900,
        },
      });
      if (res.sessionId) onSessionId?.(res.sessionId);
      setPipelineResult(res);
      if (!res.ok) {
        setError(res.error ?? "流水线中断");
        if (res.stage === "choice_pending") {
          setProgress("写作遇到抉择点，请先到上面手动处理");
          onSaved?.();
        }
        return;
      }
      setProgress(
        res.stage === "done_revised"
          ? `✅ 完成：${res.chapterFile} 已写、已审、已修订、已收尾`
          : `✅ 完成：${res.chapterFile} 已写、已审（verdict=${res.verdict}）、已收尾`,
      );
      onSaved?.();
    } catch (e) {
      setError(String(e));
    } finally {
      setPipelineRunning(false);
    }
  }, [root, instruction, sessionId, onSessionId, onSaved]);

  return (
    <div className="panel">
      <h2 className="section-title">✨ AI 写章节</h2>

      {dshLoggedIn === false && (
        <div className="external-banner">
          <span>⚠️ 未登录 DSH Web —— AI 写章节 / 审核需要先登录。</span>
          <div className="spacer" />
          <button className="btn primary" onClick={onLogin}>🔐 登录 DSH</button>
        </div>
      )}

      {dshLoggedIn && (
        <div className="toolbar" style={{ marginTop: 0 }}>
          <span className="muted small">写作模型</span>
          <select
            className="search"
            style={{ flex: 1, maxWidth: 420 }}
            value={selectedModel}
            onChange={(e) => setSelectedModel(e.target.value)}
            title="本次写作使用的模型（用完自动恢复默认，不改全局）"
          >
            <option value="">— 跟随 DSH 默认 —</option>
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
        </div>
      )}

      <textarea
        className="md-editor small"
        rows={3}
        placeholder={
          "给 AI 的指示（可空，默认按 state.yml 进度写下一章）。\n例：写第 3 章，聚焦林楚发现叛徒萧承的行踪，结尾埋下北境魔宗的伏笔。"
        }
        value={instruction}
        onChange={(e) => setInstruction(e.target.value)}
        disabled={phase === "running" || phase === "choice"}
      />

      <div className="toolbar">
        <span className="muted small">
          {sessionId ? `会话 ${sessionId.slice(0, 8)}…（保持上下文）` : "每次新建 DSH 会话"}
        </span>
        <div className="spacer" />
        <button
          className="btn primary"
          onClick={runPipeline}
          disabled={pipelineRunning || phase === "running" || phase === "choice"}
        >
          {pipelineRunning ? "流水线运行中…" : "⚡ 一键写→审→改→收尾"}
        </button>
        <button
          className="btn"
          onClick={startWrite}
          disabled={pipelineRunning || phase === "running" || phase === "choice"}
        >
          {phase === "running" ? "AI 写作中…" : "🚀 只写章节"}
        </button>
      </div>

      {progress && <p className="muted small">{progress}</p>}
      {error && <div className="error-banner">{error}</div>}

      {pipelineResult && pipelineResult.ok && (
        <div className="result-box">
          <div className="result-head">
            <span className="badge ok">流水线完成</span>
            <span className="muted small">{pipelineResult.chapterFile}</span>
          </div>
          {pipelineResult.reviewSummary && (
            <p className="review-summary-text">{pipelineResult.reviewSummary}</p>
          )}
          <div className="result-head" style={{ gap: 8 }}>
            {pipelineResult.verdict && (
              <span className={`badge ${pipelineResult.verdict === "pass" ? "ok" : "warn"}`}>
                审核：{pipelineResult.verdict === "pass" ? "通过" : "已修订"}
              </span>
            )}
            {pipelineResult.reconcileNote && (
              <span className="muted small">{pipelineResult.reconcileNote}</span>
            )}
          </div>
          <pre className="md">
            {(pipelineResult.finalText ?? "").slice(0, 1200)}
            {(pipelineResult.finalText?.length ?? 0) > 1200 ? "…（完整内容已存盘）" : ""}
          </pre>
        </div>
      )}

      {result && phase === "done" && (
        <div className="result-box">
          <div className="result-head">
            <span className="badge ok">{result.savedTo ? "已保存" : "未保存"}</span>
            {result.savedTo && (
              <>
                <button className="btn" onClick={reconcile} disabled={reconciling}>
                  {reconciling ? "同步圣经中…" : "📖 同步到圣经（收尾三件事）"}
                </button>
                <button className="btn" onClick={onSaved}>刷新看板</button>
              </>
            )}
          </div>
          {reconcileOutcome && (
            <p className={`muted small ${reconcileOutcome.startsWith("同步失败") ? "" : ""}`}>
              {reconcileOutcome}
            </p>
          )}
          <pre className="md">{result.text.slice(0, 1200)}{result.text.length > 1200 ? "…（完整内容已存盘）" : ""}</pre>
        </div>
      )}

      {choiceReq && phase === "choice" && (
        <ChoiceModal
          req={choiceReq}
          title="AI 遇到关键抉择"
          subtitle="选择会改变后续走向；也可以让 AI 自己定"
          pickedOption={pickedOption}
          note={note}
          onPick={setPickedOption}
          onNoteChange={setNote}
          onConfirm={confirmChoice}
          onCancel={cancelChoice}
          confirmLabel={pickedOption === "ai" ? "让 AI 决定并继续" : "✓ 锁定决定并继续写"}
        />
      )}
    </div>
  );
}
