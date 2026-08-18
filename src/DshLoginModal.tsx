import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DshLoginResult } from "./types";

interface DshLoginModalProps {
  onClose: () => void;
  onLoggedIn: () => void;
}

/** DSH Web 登录框：账号密码（+ 可选 TOTP 二步），登录后保存 session cookie。 */
export default function DshLoginModal({ onClose, onLoggedIn }: DshLoginModalProps) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [mfaToken, setMfaToken] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await invoke<DshLoginResult>("dsh_login", {
        args: {
          username,
          password,
          code: mfaToken ? code.trim() || null : null,
          mfaToken,
        },
      });
      if (res.ok) {
        onLoggedIn();
        onClose();
        return;
      }
      if (res.mfaRequired && res.mfaToken) {
        // 进入二步验证
        setMfaToken(res.mfaToken);
        setCode("");
        setError(null);
        return;
      }
      setError(res.message || "登录失败");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-overlay">
      <div className="modal">
        <div className="modal-header">
          <h2>🔐 登录 DSH Web</h2>
          <button className="btn ghost" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body">
          <p className="form-hint" style={{ margin: 0 }}>
            桌面端驱动 AI 写章节 / 审核需要先登录 DSH Web（127.0.0.1:3080）。
            登录成功后会自动记住会话，之后无需重复登录。
          </p>

          {mfaToken ? (
            <div className="form-grid" style={{ marginTop: 12 }}>
              <label className="label">二步验证码</label>
              <input
                className="search"
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder="6 位 TOTP 码（或备份码）"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                autoFocus
              />
              <p className="form-hint">从你的验证器 App（Google Authenticator / 1Password…）取当前码。</p>
            </div>
          ) : (
            <div className="form-grid" style={{ marginTop: 12 }}>
              <label className="label">用户名</label>
              <input
                className="search"
                type="text"
                autoComplete="username"
                placeholder="DSH Web 登录用户名"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoFocus
              />
              <label className="label">密码</label>
              <input
                className="search"
                type="password"
                autoComplete="current-password"
                placeholder="密码"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
          )}

          {error && <div className="error-banner" style={{ marginTop: 12 }}>{error}</div>}
        </div>
        <div className="modal-foot">
          <button className="btn" onClick={onClose}>取消</button>
          <button
            className="btn primary"
            onClick={submit}
            disabled={
              busy ||
              (mfaToken ? !code.trim() : (!username.trim() || !password))
            }
          >
            {busy ? "登录中…" : mfaToken ? "验证并登录" : "登录"}
          </button>
        </div>
      </div>
    </div>
  );
}
