# novel-studio-desktop

写作工作台 — 一个轻量 **Tauri 2** 桌面客户端，专门为 long-form fiction 设计，**直接读取 `agt novel-init` 生成的 bible/chapters/state.yml**，完全不需要 DSH Web 启动、不需要占用模型端口。

## 跟 dsh-cockpit / dsh-novel-studio 的关系

- **dsh-cockpit**（`sandyhery/dsh-cockpit`）= 插件市场管理客户端，智驾仓
- **dsh-novel-studio** = DSH Web GUI 内的「小说工作台」侧栏插件（host 工具 + client 源文件）
- **novel-studio-desktop**（**本仓库**）= 独立的写作桌面应用，不依赖前两者，能在 DSH 不运行时照常读写小说项目

跟 `dsh-novel-studio` 共享数据格式（基于 `agt novel-init` 生成的 `state.yml` 与 markdown 圣经），但 UI 层完全独立。

## 4 个核心能力

| 能力 | 来源 |
|---|---|
| 章节看板 | chapters/*.md 自动列出（按自然序、显示字数、改时间） |
| 圣经概览 | state.yml + bible/ 全列，含角色 |
| 章节编辑 | textarea 编辑器，写入即持久化（原子覆盖） |
| 写作沉浸模式 | 单窗口 macOS 原生体验 |

## 数据流

```
AGT novel-init 生成的小说项目目录
   ├── state.yml          ← 项目总览
   ├── bible/
   │   ├── timeline.md          ← 待写
   │   ├── foreshadowing.md     ← 已知伏笔
   │   ├── locations.md         ← 场景
   │   └── characters/林楚.md    ← 角色档案
   └── chapters/
       ├── ch01.md              ← 章节正文
       ├── ch02.md
       └── ...
                │
                ▼
   novel-studio-desktop 5 条 Tauri 命令：
   - read_summary(root)        → 总览 JSON
   - read_bible(root, file)    → 读圣经全文
   - read_chapter(root, file)  → 读章节
   - write_chapter(args)       → 保存章节
                │
                ▼
   React + TS 渲染：4 个面板（概览/章节/圣经/编辑器）
```

## 开发

```sh
pnpm install
pnpm tauri dev          # 热更新（首次会编译）
```

打包：
```sh
pnpm tauri build        # 产物 .app / .dmg（macOS）
```

> 跟 dsh-cockpit 一样，依赖 `pnpm` 在 PATH 上。

## Tauri 命令速查

```ts
invoke<NovelSummary>("read_summary", { root })      // 总览
invoke<string>("read_bible", { root, file })        // 圣经全文
invoke<string>("read_chapter", { root, file })      // 章节全文
invoke("write_chapter", { args: { root, file, content } })  // 保存
```

文件路径限制：
- `chapters/` 文件名必须 `.md` 结尾，不含 `/` 和 `..`
- `bible/` 文件名不带 `.md` 后缀传入，例： `"timeline"`、`"characters/林楚"`

## 不做的事（核心约束）

- ❌ 不依赖 `dsh web` 是否运行
- ❌ 不直接调模型（不发起 LLM 请求）；让模型写章节，你回 DSH Web 那边
- ❌ 不修改小说项目的 markdown 格式（仅覆盖 chapters/）
- ❌ 不上传/同步任何内容（一切本地）

## Roadmap（按主线优先级）

1. 章节编辑器 → 已实现
2. 章节看板 → 已实现
3. 世界圣经只读浏览 → 已实现
4. 写作中字体、行距切换 → 已实现（字号 14~22 / 行距 4 档，localStorage 记忆）
5. 阅读模式 / 全屏沉浸模式 → 已实现（Markdown 排版阅读 + 隐藏界面全屏沉浸）
6. 章节字数目标达成 banner → 已实现（按单章建议字数显示进度与达标提示）
7. 写满 N 章后自动 git commit → 已实现（每 N 章里程碑自动快照 + 手动提交 + 一键初始化 Git）
8. 与 dsh-novel-studio 的 read/write API 双向打通（同一份小说双端可编辑）→ 已实现核心（写前脏检查 + 冲突裁决 + 文件变更自动刷新；设计见 `docs/interop-design.md`）
