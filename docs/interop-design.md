# 与 dsh-novel-studio 双向打通 — 设计（#8）

> 状态：设计稿，待评审。目标：同一份小说（`state.yml` + `bible/` + `chapters/`）在
> 桌面端 `novel-studio-desktop` 与 DSH Web GUI 插件 `dsh-novel-studio` 之间**双向可编辑、互不丢改动**。

## 0. 关键结论（先读）

两边**已经共享同一份文件系统数据**，且 markdown/yml 约定一致：

| 面 | 读 | 写 |
|---|---|---|
| 桌面端 | `read_summary/read_bible/read_chapter` 直读文件 | `write_chapter` 覆盖章节；`ai_reconcile_bible` 定点插入 bible |
| 插件 host | 6 个 `novel_*` 工具直读文件；`GET /novel-studio/api/summary`、`/bible`（仅 GET、仅 loopback） | `novel_character_update` / `novel_timeline_append` / `novel_foreshadow_update` 追加/定点替换 |

关键事实：
- 插件的 HTTP 路由是**只读**的（summary / bible），没有写路由——插件写操作是模型工具内部行为，走文件系统。
- 两边的 `readState` 字段兼容（`title / current_chapter / world_date / pov / next_hook / foreshadowing_open`）。
- 两边的 bible 写都在「（追加）」占位行前插入，不会互相破坏表格结构。
- 桌面端已有 `dsh_client.rs`（JSON-RPC 驱动 DSH agent）与 `ping()`。

所以 #8 的缺口**不在数据格式**，而在：

1. **同步感知** —— 桌面端不知道外部（DSH agent / 插件 / 任意编辑器）改过文件，看板会过期。
2. **覆盖冲突** —— 桌面端 `write_chapter` 整文件覆盖；若你在编辑期间 DSH agent 改写了同一章，点保存就静默丢掉对方改动。
3. **能力不对等** —— 插件有 `novel_check`（一致性冲突检测）与 `novel_refine`（记忆分层精炼），桌面端没有。

## 1. 设计原则

- **文件系统即契约**：不把桌面端写操作改走 DSH 的 HTTP API（那会重新引入对 DSH Web 的依赖，违背桌面端「独立运行」的初衷）。
- **零额外依赖**：沿用项目「手写轻量」惯例，用轮询 + 指纹，不引入 `notify` 等重依赖。
- **绝不静默丢失**：任何「磁盘版本 vs 内存草稿」不一致，都交给用户裁决。

## 2. 方案（三块，按优先级）

### B. 写前脏检查 + 冲突提示（安全，优先级最高）

- 前端打开章节时记录「base 指纹」：`mtime_ms + size`（或内容 hash，见 §3）。
- `write_chapter` 增加可选参数 `baseFingerprint`：后端保存前比对当前磁盘指纹，不一致则返回冲突结果，不写盘。
- 前端收到冲突 → 弹窗：「磁盘版本已在你编辑期间被修改」→ 三个选择：
  1. **载入磁盘版本**（放弃草稿，刷新看板）；
  2. **仍用我的版本覆盖**（用户显式确认，再写盘）；
  3. **取消**（继续编辑草稿，稍后再处理）。
- 这样双向编辑时**不会静默丢改动**。

### A. 文件变更自动刷新（同步）

- 后端新增一个**廉价**命令 `read_changes(root, lastSeen)`：只 `stat` chapters/、bible/、state.yml 的 `mtime/size`，返回 `changed: bool` + 变更路径列表（不读文件内容，开销小）。
- 前端 `setInterval`（默认 2s，窗口失焦暂停）调 `read_changes`，有变化才触发一次完整 `read_summary` 刷新看板。
- 编辑中的章节若被外部改动：编辑器顶部出「⚠ 该文件已在别处被修改」横幅 +「重载磁盘版本」按钮（不自动覆盖你的草稿）。
- 效果：DSH 里写 → 桌面端几秒内看到；桌面端写 → DSH 下次工具调用自然读到（DSH 每次都是现读文件，无需额外处理）。

### C. 能力对齐 + DSH 状态（parity，可选小项）

- 桌面端新增「一致性检测」「记忆精炼」两个入口，直接 shell 到 `agt novel-check <root>` / `agt refine <root>`（不依赖 DSH Web），与插件的 `novel_check`/`novel_refine` 对齐。
- 顶部显示「DSH Web 运行中 / 未运行」状态徽标（复用现有 `dsh_client::ping`），让用户知道 AI 写章节 / 审核是否可用。

## 3. 关键技术细节

**指纹**：优先用 `(mtime_ms, size)`，简单、无需读全文；若担心 size 相同的改动，用「mtime + 首尾各 1KB 内容 hash」。初版用 `mtime_ms + size` 即可，够用。

**轮询 vs 文件监听**：轮询实现简单、跨平台一致、无新依赖；2s 间隔对写作场景足够。若将来觉得不够，再换 `notify` crate（纯 Rust）。

**后端新命令清单**：
- `write_chapter` 增 `base_fingerprint: Option<Fingerprint>`（向后兼容，旧调用不传则跳过检查）。
- `read_changes(root, last_seen: Map<path, (mtime,size)>) -> { changed, paths }`。
- （可选 C）`novel_check(root)` / `novel_refine(root, note)` 命令（shell 到 agt，`timeout 30s/45s`）。

## 4. 实施顺序与涉及文件

1. **B**（脏检查 + 冲突弹窗）—— 改动集中在 `write_chapter` + `EditorPanel`。
2. **A**（`read_changes` + 自动刷新 + 外部改动横幅）。
3. **C**（parity，小项，可砍）。

涉及文件：
- `src-tauri/src/lib.rs`：`write_chapter` 加指纹参数 + `read_changes`（+ 可选 `novel_check`/`novel_refine`）。
- `src/App.tsx`：编辑器指纹记录、冲突弹窗、自动刷新 effect、外部改动横幅、DSH 状态徽标。
- `src/types.ts`：`Fingerprint` / `ChangeDelta` / 冲突结果 / DSH 状态类型。
- `src/App.css`：冲突弹窗、外部改动横幅、DSH 徽标样式。
- （可选）`src-tauri/src/dsh_client.rs`：复用 `ping`，无需改。

## 5. 明确不做

- ❌ 桌面端写操作改走 DSH HTTP API（重新引入依赖，违背初衷）。
- ❌ 跨进程文件锁 / 复杂并发协议（写作场景冲突频率低，脏检查 + 用户裁决足够）。
- ❌ 改任何 markdown/yml 格式（保持与 `agt novel-init` 100% 兼容）。
