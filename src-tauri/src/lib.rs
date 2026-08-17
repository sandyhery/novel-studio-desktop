//! 小说工作台 — host 侧
//!
//! 完全独立于 DSH / dsh-novel-studio：直接读取 `agt novel-init` 生成的
//! `bible/`、`chapters/`、`state.yml`。这样 Tauri 应用不需要 DSH Web
//! 启动、不占用模型端口、也跟 DSH 框架版本解耦。
//!
//! 设计原则：
//! - 只读好。世界圣经是模型写的，Tauri 这边只展示 + 让用户写章节。
//! - state.yml 解析够用就好（6~8 个键、手写）。
//! - 一切错误返回 Result<T, String>，前端可读。
//!
//! Tauri 命令清单（5 个，全部只读）：
//! - pick_root()                  让用户选目录（走 dialog plugin，前端触发）
//! - read_summary(root)           项目状态总览
//! - read_bible(root, file)       读圣经某文件全文
//! - read_chapter(root, file)     读章节全文
//! - write_chapter(root, file, c) 保存章节

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 类型
// ---------------------------------------------------------------------------

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BibleFile {
    /// 不含 .md 后缀
    pub name: String,
    /// 完整路径
    pub path: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    /// 不含 .md 后缀
    pub name: String,
    /// 完整路径
    pub path: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BibleMeta {
    pub files: Vec<BibleFile>,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub file: String,
    /// 文件名（带 .md）— 用户后续保存用同一个 key
    pub path: String,
    /// 章节标题（从正文首行 # 解析，缺省用文件名）
    pub title: String,
    /// 文件字节数
    pub bytes: u64,
    /// 中文字数（去掉空白后字符数）
    pub chars: u64,
    /// 开头摘要
    pub head: String,
    /// 修改时间（Unix millis）
    pub modified_ms: u64,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NovelState {
    pub title: String,
    pub current_chapter: String,
    pub world_date: String,
    pub pov: String,
    pub next_hook: String,
    /// 待回收伏笔条数
    pub foreshadowing_open: u64,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NovelSummary {
    pub ok: bool,
    pub root: String,
    pub title: String,
    pub state: NovelState,
    pub bible: BibleMeta,
    pub chapters: Vec<Chapter>,
    /// 最近修改的 5 个章节（按 mtime 倒序）
    pub recent_chapters: Vec<Chapter>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NovelError {
    pub ok: bool,
    pub error: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteChapterArgs {
    pub root: String,
    pub file: String,
    pub content: String,
}

/// 新建小说项目的全部要素（向导一次性收集，前端确认后整批下发）。
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CreateNovelArgs {
    /// 父目录（项目目录会创建在这里）
    pub parent: String,
    /// 项目名（同时作为目录名与默认标题的种子）
    pub name: String,
    pub title: String,
    /// 视角：第一人称 / 第三人称 / 多重视角
    pub pov_mode: String,
    /// POV 主角名（povMode=多重视角时可空）
    pub pov_character: String,
    /// 类型（玄幻 / 都市 / 科幻 / 言情 / 历史 / 悬疑 / 其他）
    pub genre: String,
    /// 基调（轻松 / 严肃 / 黑暗 / 爽文 / 治愈）
    pub tone: String,
    /// 时代背景（架空 / 现代 / 未来 / 历史朝代名）
    pub era: String,
    /// 字数目标（万字）
    pub target_words_wan: u32,
    /// 卷数
    pub volumes: u32,
    /// 每卷章数
    pub chapters_per_volume: u32,
    /// 核心矛盾一句话
    pub core_conflict: String,
    /// 主角处境一句话
    pub hero_situation: String,
    /// 主角欲望一句话
    pub hero_desire: String,
    /// 第一幕核心冲突一句话
    pub opening_hook: String,
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

fn is_novel_root(dir: &Path) -> bool {
    dir.join("state.yml").is_file() && dir.join("bible").is_dir()
}

/// 从 root 向上找最近的 agt novel-init 项目根目录，None 表示没找到。
fn find_novel_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    for _ in 0..8 {
        if is_novel_root(&cur) {
            return Some(cur);
        }
        let parent = cur.parent()?;
        if parent == cur {
            return None;
        }
        cur = parent.to_path_buf();
    }
    None
}

fn err_to_string<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}

fn read_state(root: &Path) -> NovelState {
    let mut state = NovelState::default();
    let path = root.join("state.yml");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return state,
    };
    // 手写最简 YAML：每行 `key: value`（值可以是带引号或不带、空行 / # 注释跳过）
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let key = k.trim();
        // 剥掉值两端的引号 + 行内注释
        let v = v
            .trim()
            .split_once("  #")
            .map(|(head, _)| head)
            .unwrap_or(v)
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        match key {
            "title" => state.title = v.to_string(),
            "current_chapter" => state.current_chapter = v.to_string(),
            "world_date" => state.world_date = v.to_string(),
            "pov" => state.pov = v.to_string(),
            "next_hook" => state.next_hook = v.to_string(),
            _ => {}
        }
    }
    // 兜底：标题用目录名
    if state.title.is_empty() {
        if let Some(name) = root.file_name().and_then(|s| s.to_str()) {
            state.title = name.to_string();
        }
    }
    state
}

fn char_count_zh(text: &str) -> u64 {
    // 去掉所有空白（包括换行）后按字符数算；中文按字算
    text.chars().filter(|c| !c.is_whitespace()).count() as u64
}

fn first_heading(text: &str) -> String {
    for line in text.lines() {
        let trimmed = line.trim_start_matches('#').trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    String::new()
}

fn list_bible(root: &Path) -> std::io::Result<BibleMeta> {
    let mut meta = BibleMeta::default();
    let bible_dir = root.join("bible");
    for entry in fs::read_dir(&bible_dir)? {
        let entry = entry?;
        let p = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if p.is_file() && file_name.ends_with(".md") {
            meta.files.push(BibleFile {
                name: file_name.trim_end_matches(".md").to_string(),
                path: p.to_string_lossy().to_string(),
            });
        } else if p.is_dir() && file_name == "characters" {
            for ch in fs::read_dir(&p)? {
                let ch = ch?;
                let cp = ch.path();
                let cname = ch.file_name().to_string_lossy().to_string();
                if cp.is_file() && cname.ends_with(".md") && cname != "_template.md" {
                    meta.characters.push(Character {
                        name: cname.trim_end_matches(".md").to_string(),
                        path: cp.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }
    // 排序
    meta.files.sort_by(|a, b| a.name.cmp(&b.name));
    meta.characters.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(meta)
}

fn list_chapters(root: &Path) -> std::io::Result<Vec<Chapter>> {
    let mut out = Vec::new();
    let dir = root.join("chapters");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.ends_with(".md") {
            continue;
        }
        let path = entry.path();
        let meta = entry.metadata().ok();
        let text = fs::read_to_string(&path).unwrap_or_default();
        let mut chap = Chapter {
            file: file_name.clone(),
            path: path.to_string_lossy().to_string(),
            title: first_heading(&text),
            bytes: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            chars: char_count_zh(&text),
            head: text.lines().take(3).collect::<Vec<_>>().join(" ").chars().take(80).collect(),
            modified_ms: meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        if chap.title.is_empty() {
            chap.title = file_name;
        }
        out.push(chap);
    }
    // 自然排序（让 ch1.md / ch2.md / ch10.md 顺序正确）
    out.sort_by(|a, b| natural_cmp(&a.file, &b.file));
    Ok(out)
}

/// 自然排序：ch2 < ch10 < ch100
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    while let (Some(ac), Some(bc)) = (ai.peek(), bi.peek()) {
        if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let mut an = String::new();
            while let Some(c) = ai.peek() {
                if c.is_ascii_digit() {
                    an.push(*c);
                    ai.next();
                } else {
                    break;
                }
            }
            let mut bn = String::new();
            while let Some(c) = bi.peek() {
                if c.is_ascii_digit() {
                    bn.push(*c);
                    bi.next();
                } else {
                    break;
                }
            }
            let an: u128 = an.parse().unwrap_or(0);
            let bn: u128 = bn.parse().unwrap_or(0);
            match an.cmp(&bn) {
                std::cmp::Ordering::Equal => continue,
                ord => return ord,
            }
        }
        match ac.cmp(bc) {
            std::cmp::Ordering::Equal => {
                ai.next();
                bi.next();
            }
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}

fn count_open_foreshadow(root: &Path) -> u64 {
    let path = root.join("bible").join("foreshadowing.md");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    // 简单统计"含待收伏笔"的中格
    text.lines()
        .filter(|l| l.starts_with('|') && l.contains("待收"))
        .count() as u64
}

fn title_from_state_or_dir(root: &Path, state_title: &str) -> String {
    if !state_title.is_empty() {
        state_title.to_string()
    } else {
        root.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("(未命名)")
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Probe — 对一个路径的"诊断"，用于前端友好提示
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ProbeKind {
    /// 路径不存在（没创建过）
    Missing,
    /// 路径是个文件不是目录
    FileNotDir,
    /// 是空目录 —— 可以直接"原地初始化"成 novel-init
    EmptyDir,
    /// 是非空目录且有启发性内容，但不算 novel-init 项目
    NonEmptyDir {
        /// 头几条目名（最多 5），帮助用户识别这个目录是什么
        sample: Vec<String>,
    },
    /// 是当前 novel 项目子目录（chapters/、bible/characters/xxx.md 之类）
    NovelSubdir { root: String },
    /// 就是 novel 项目根
    NovelRoot,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub kind: ProbeKind,
    /// 用户选的那个路径
    pub path: String,
    /// 父目录（用于"在这里新建"按钮）
    pub parent: String,
    /// 如果是 Missing 或 EmptyDir，目录名建议给向导预填
    pub suggested_name: String,
}

/// 诊断一个路径：帮助前端决定显示什么提示 + 哪几个动作按钮
#[tauri::command]
fn probe_directory(path: String) -> Result<ProbeResult, String> {
    let p = PathBuf::from(&path);
    let parent = p
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let suggested_name = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("novel")
        .to_string();

    if !p.exists() {
        return Ok(ProbeResult {
            kind: ProbeKind::Missing,
            path: p.to_string_lossy().to_string(),
            parent,
            suggested_name,
        });
    }
    if !p.is_dir() {
        return Ok(ProbeResult {
            kind: ProbeKind::FileNotDir,
            path: p.to_string_lossy().to_string(),
            parent,
            suggested_name,
        });
    }
    // 是不是 novel 项目根？
    if is_novel_root(&p) {
        return Ok(ProbeResult {
            kind: ProbeKind::NovelRoot,
            path: p.to_string_lossy().to_string(),
            parent,
            suggested_name,
        });
    }
    // 是不是某个祖先链上的 novel 项目？
    if let Some(root) = find_novel_root(&p) {
        return Ok(ProbeResult {
            kind: ProbeKind::NovelSubdir {
                root: root.to_string_lossy().to_string(),
            },
            path: p.to_string_lossy().to_string(),
            parent,
            suggested_name,
        });
    }
    // 不是 novel 项目。看是不是空目录
    let mut entries: Vec<String> = fs::read_dir(&p)
        .map_err(|e| format!("read_dir {}: {e}", p.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != ".DS_Store")
        .collect();
    if entries.is_empty() {
        return Ok(ProbeResult {
            kind: ProbeKind::EmptyDir,
            path: p.to_string_lossy().to_string(),
            parent,
            suggested_name,
        });
    }
    entries.sort();
    entries.truncate(5);
    Ok(ProbeResult {
        kind: ProbeKind::NonEmptyDir { sample: entries },
        path: p.to_string_lossy().to_string(),
        parent,
        suggested_name,
    })
}

/// 读项目总览：state.yml + bible/ + chapters/，一次性拿全。
#[tauri::command]
fn read_summary(root: String) -> Result<NovelSummary, String> {
    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start).ok_or_else(|| {
        format!(
            "未找到小说项目（{} 上级目录都没找到 state.yml + bible/）。先运行 `agt novel-init <目录>` 搭建。",
            start.display()
        )
    })?;
    let mut state = read_state(&novel_root);
    let bible = list_bible(&novel_root).map_err(err_to_string)?;
    let mut chapters = list_chapters(&novel_root).map_err(err_to_string)?;
    let open_fs = count_open_foreshadow(&novel_root);
    state.foreshadowing_open = open_fs;

    // 最近修改的 5 章
    let mut recent = chapters.clone();
    recent.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    recent.truncate(5);

    let title = title_from_state_or_dir(&novel_root, &state.title);

    Ok(NovelSummary {
        ok: true,
        root: novel_root.to_string_lossy().to_string(),
        title,
        state,
        bible,
        chapters: std::mem::take(&mut chapters),
        recent_chapters: recent,
    })
}

/// 读某圣经文件 markdown 全文（file 不带 .md，例如 "timeline" 或 "characters/林楚"）。
#[tauri::command]
fn read_bible(root: String, file: String) -> Result<String, String> {
    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let bible_dir = novel_root.join("bible");
    let path = if file.starts_with("characters/") {
        let name = file.trim_start_matches("characters/");
        bible_dir.join("characters").join(format!("{name}.md"))
    } else {
        bible_dir.join(format!("{file}.md"))
    };
    fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// 读某章节 markdown 全文（file 是文件名，例如 "ch01.md"）。
#[tauri::command]
fn read_chapter(root: String, file: String) -> Result<String, String> {
    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let path = novel_root.join("chapters").join(&file);
    fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// 保存一个章节（覆盖写）。防呆：file 必须不含 `/`，且必须 .md 后缀。
#[tauri::command]
fn write_chapter(args: WriteChapterArgs) -> Result<(), String> {
    if args.file.contains('/')
        || args.file.contains("..")
        || !args.file.ends_with(".md")
    {
        return Err("非法的章节文件名".into());
    }
    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let path = novel_root.join("chapters").join(&args.file);
    fs::write(&path, args.content.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))
}

/// 新建小说项目：
/// 1. 在 `parent` 下创建目录 `name`（如已存在则报错）
/// 2. 调用 `agt novel-init <path> --title <title>` 生成骨架
/// 3. 写入 `bible/world-rules.md`（题材/规模/开局要素）
/// 4. 覆盖更新 `state.yml`（pov / world_date / next_hook 等）
/// 5. 返回新建后的项目根路径
#[tauri::command]
fn create_novel(args: CreateNovelArgs) -> Result<String, String> {
    use std::process::Command;

    // 防呆：name 必须是目录名合法字符
    if args.name.is_empty()
        || args.name.contains('/')
        || args.name.contains('\\')
        || args.name == "."
        || args.name == ".."
    {
        return Err("非法项目名（不能含 /，不能是 . / ..）".into());
    }
    if args.title.is_empty() {
        return Err("标题不能为空".into());
    }
    if args.parent.is_empty() {
        return Err("请先选父目录".into());
    }

    let parent = PathBuf::from(&args.parent);
    if !parent.is_dir() {
        return Err(format!("父目录不存在：{}", parent.display()));
    }
    let project_dir = parent.join(&args.name);
    if project_dir.exists() {
        return Err(format!(
            "目标目录已存在：{}（换一个项目名或选别的父目录）",
            project_dir.display()
        ));
    }
    fs::create_dir_all(&project_dir)
        .map_err(|e| format!("创建目录失败 {}: {e}", project_dir.display()))?;

    // 调 agt novel-init（同步等待；超时 30s）
    let init_out = Command::new("agt")
        .arg("novel-init")
        .arg(&project_dir)
        .arg("--title")
        .arg(&args.title)
        .output();
    let init_out = match init_out {
        Ok(o) => o,
        Err(e) => {
            // 清理半成品
            let _ = fs::remove_dir_all(&project_dir);
            return Err(format!(
                "调用 agt novel-init 失败：{}（确保 PATH 上有 agt）",
                e
            ));
        }
    };
    if !init_out.status.success() {
        let _ = fs::remove_dir_all(&project_dir);
        return Err(format!(
            "agt novel-init 退出码 {:?}：stderr={}",
            init_out.status.code(),
            String::from_utf8_lossy(&init_out.stderr)
        ));
    }

    // 1. 写 world-rules.md（题材 / 规模 / 第一幕冲突的固化文档）
    let total_chapters = args.volumes * args.chapters_per_volume;
    let target_words: u64 = u64::from(args.target_words_wan) * 10_000;
    let world_rules = format!(
        "# 世界规则（题材与规模定稿）\n\n\
         > 一旦写下即为定稿，正文引用本文件，不要在正文里另造。\n\n\
         ## 题材\n\
         - 类型：{genre}\n\
         - 基调：{tone}\n\
         - 时代背景：{era}\n\n\
         ## 规模\n\
         - 目标字数：{target_wan} 万字（约 {target_words} 字）\n\
         - 卷数：{volumes}\n\
         - 总章数：{total_ch} 章（每卷约 {cpv} 章）\n\
         - 单章建议字数：约 {per_ch} 字（=目标字数/总章数）\n\n\
         ## 视角\n\
         - 视角：{pov_mode}\n\
         - POV 主角：{pov_character}\n\n\
         ## 开局要素\n\
         - 核心矛盾：{core_conflict}\n\
         - 主角处境：{hero_situation}\n\
         - 主角欲望：{hero_desire}\n\
         - 第一幕核心冲突：{opening_hook}\n\n\
         ## 写作约束（自动派发）\n\
         - **不许在正文里改设定**：角色、时间线、伏笔、世界规则只能改本档案，\n\
           正文引用档案。本文件 = 单一源。\n\
         - **每章收尾三件事**：角色状态回流 → `timeline.md` 追加 → `foreshadowing.md` 登记/销账\n\
         - **回读一致性**：写完后回读本章涉及的角色档案与 timeline，确认无漂移\n",
        genre = args.genre,
        tone = args.tone,
        era = args.era,
        target_wan = args.target_words_wan,
        target_words = target_words,
        volumes = args.volumes,
        total_ch = total_chapters,
        cpv = args.chapters_per_volume,
        per_ch = if total_chapters > 0 {
            target_words / u64::from(total_chapters)
        } else {
            0
        },
        pov_mode = args.pov_mode,
        pov_character = if args.pov_character.is_empty() {
            "（多重视角）".to_string()
        } else {
            args.pov_character.clone()
        },
        core_conflict = args.core_conflict,
        hero_situation = args.hero_situation,
        hero_desire = args.hero_desire,
        opening_hook = args.opening_hook,
    );
    let bible_dir = project_dir.join("bible");
    fs::write(bible_dir.join("world-rules.md"), world_rules.as_bytes())
        .map_err(|e| format!("写 bible/world-rules.md 失败: {e}"))?;

    // 2. 重写 state.yml（保留原注释/结构，按键覆盖）
    let today = chrono_like_today();
    let state_content = format!(
        "# 当前进度（每章更新）\n\n\
         title: {title}\n\
         current_chapter: 0        # 下一章编号（已写到 ch{{N}} → 填 N+1）\n\
         world_date: \"{era} _ 第 N 章开篇\"     # 第一章开篇时填具体时间\n\
         pov: \"{pov_char_for_state}\"           # 本章视角角色\n\
         scene_characters: []      # 本章在场角色（开写前定，写后校准）\n\
         next_hook: \"{opening_hook}\"\n\
         done_chapters:\n\
           - \"ch0: 大纲/设定\"\n\
         blocked: []               # 卡点\n\
         foreshadowing_open: 1     # 开篇埋伏笔 F001（见 bible/foreshadowing.md）\n\
         updated: \"{today}\"\n",
        title = args.title,
        era = args.era,
        pov_char_for_state = if args.pov_character.is_empty() {
            "（多重视角）".to_string()
        } else {
            args.pov_character.clone()
        },
        opening_hook = args.opening_hook,
        today = today,
    );
    fs::write(project_dir.join("state.yml"), state_content.as_bytes())
        .map_err(|e| format!("写 state.yml 失败: {e}"))?;

    // 3. 在 bible/foreshadowing.md 登记 F001：根据 opening_hook 包一行种子
    let foreshadow_path = bible_dir.join("foreshadowing.md");
    if let Ok(existing) = fs::read_to_string(&foreshadow_path) {
        // 在表格里 `（追加）` 占位行前插一行
        if let Some(idx) = existing.find("（追加）") {
            let mut lines: Vec<&str> = existing.lines().collect();
            let new_row = format!(
                "| F001 | ch1 | {} | 开局埋设 | 埋设 | — |",
                truncate_for_table_cell(&args.opening_hook, 40)
            );
            lines.insert(idx, new_row.as_str());
            let updated = lines.join("\n");
            fs::write(&foreshadow_path, updated.as_bytes())
                .map_err(|e| format!("写 foreshadowing.md 失败: {e}"))?;
        }
    }

    Ok(project_dir.to_string_lossy().to_string())
}

/// 取本地日期（不要引 chrono 库，用 std 拿）
fn chrono_like_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;
    // 简单 days -> year/month/day 换算（Gregorian proleptic，不用 chrono）
    let days = secs / 86_400;
    let mut year = 1970;
    let mut remaining = days;
    loop {
        let leap = is_leap(year);
        let yd = if leap { 366 } else { 365 };
        if remaining < yd {
            break;
        }
        remaining -= yd;
        year += 1;
    }
    let leap = is_leap(year);
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1;
    for m in &months {
        if remaining < *m {
            break;
        }
        remaining -= *m;
        month += 1;
    }
    let day = remaining + 1;
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// 把一段中文文本压缩成表格单元（去除换行 / 多余空格，截断到 n 字）
fn truncate_for_table_cell(s: &str, max_chars: usize) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() || *c == '_')
        .collect();
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let mut out: String = cleaned.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// entry
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            read_summary,
            read_bible,
            read_chapter,
            write_chapter,
            create_novel,
            probe_directory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    //! 集成路径探测，覆盖真实场景：用户选了空目录 / 项目子目录 / 不存在的路径。
    //! 注意 macOS 会在新目录里立即建一个 `.DS_Store`，所以"空目录"判定要容忍它。

    use super::*;
    use std::fs;

    #[test]
    fn probe_missing_path() {
        let path = std::env::temp_dir().join("dsh-novel-probe-missing-xyz-987");
        let _ = fs::remove_dir_all(&path);
        let r = probe_directory(path.to_string_lossy().to_string()).expect("call ok");
        assert!(matches!(r.kind, ProbeKind::Missing), "got {:?}", r.kind);
        assert!(!r.suggested_name.is_empty());
    }

    #[test]
    fn probe_empty_dir_or_non_empty_dir() {
        let p = std::env::temp_dir().join("dsh-novel-probe-empty-12345");
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        let r = probe_directory(p.to_string_lossy().to_string()).expect("call ok");
        // macOS 通常会立刻写一个 .DS_Store；两种结果都接受
        match r.kind {
            ProbeKind::EmptyDir | ProbeKind::NonEmptyDir { .. } => {}
            other => panic!("got {:?}", other),
        }
        let _ = fs::remove_dir_all(&p);
    }

    #[test]
    fn probe_novel_root() {
        let p = std::env::temp_dir().join("dsh-novel-probe-real-novel");
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        fs::create_dir_all(p.join("bible")).unwrap();
        fs::write(p.join("state.yml"), "title: t\ncurrent_chapter: 0\n").unwrap();
        let r = probe_directory(p.to_string_lossy().to_string()).expect("call ok");
        assert!(matches!(r.kind, ProbeKind::NovelRoot), "got {:?}", r.kind);
        assert_eq!(r.suggested_name, "dsh-novel-probe-real-novel");
        let _ = fs::remove_dir_all(&p);
    }

    #[test]
    fn probe_novel_subdir() {
        let root = std::env::temp_dir().join("dsh-novel-probe-subdir-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("chapters").join("draft")).unwrap();
        fs::create_dir_all(root.join("bible")).unwrap();
        fs::write(root.join("state.yml"), "title: t\ncurrent_chapter: 0\n").unwrap();
        let sub = root.join("chapters").join("draft");
        let r = probe_directory(sub.to_string_lossy().to_string()).expect("call ok");
        match r.kind {
            ProbeKind::NovelSubdir { root: found } => {
                assert!(found.ends_with("dsh-novel-probe-subdir-root"), "found={:?}", found);
            }
            other => panic!("got {:?}", other),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_nonexistent_chinese_path() {
        let p = "/Users/zhouluyong/Documents/我的小说-does-not-exist-测试";
        let r = probe_directory(p.to_string()).expect("call ok");
        assert!(matches!(r.kind, ProbeKind::Missing), "got {:?}", r.kind);
        assert_eq!(r.suggested_name, "我的小说-does-not-exist-测试");
    }

    #[test]
    fn probe_real_user_novel_dir() {
        // 用户之前一直在用的真实目录 /Users/zhouluyong/Documents/我的小说
        // 我们之前用 agt novel-init 把它实化成了项目根，应该被识别为 NovelRoot。
        // 如果这一项失败，意味着用户端的状态不一致 —— 用 ensure_agt_novel_init 修复。
        let p = "/Users/zhouluyong/Documents/我的小说";
        let r = probe_directory(p.to_string()).expect("call ok");
        assert!(matches!(r.kind, ProbeKind::NovelRoot), "got {:?}", r.kind);
    }
}
