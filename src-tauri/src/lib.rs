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
//!
//! 新增（抉择点 / DSH agent 集成）：
//! - read_choice_points(root) / decide_choice_point / create_choice_point / seed_demo_choice_points
//! - ai_write_chapter(root, instruction)  驱动 DSH agent 写一章（含抉择点暂停）

mod dsh_client;

use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    /// 单章建议字数目标（0 = 未知，不显示达标 banner）
    pub chapter_target_words: u64,
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
    /// 打开章节时记录的文件指纹；保存时若磁盘版本已变，返回 conflict 而不写盘。
    /// 传 None 表示强制覆盖（跳过脏检查）。
    pub base_fingerprint: Option<Fingerprint>,
}

/// 文件指纹：mtime（Unix 毫秒）+ 字节数。用于双向打通时的写前脏检查。
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub mtime_ms: u64,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteChapterResult {
    pub ok: bool,
    pub conflict: bool,
    /// 写盘后的最新指纹（成功时），或磁盘当前指纹（冲突时）。
    pub mtime_ms: u64,
    pub size: u64,
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
            "chapter_target_words" => {
                state.chapter_target_words = v.parse().unwrap_or(0);
            }
            _ => {}
        }
    }
    // 兜底：标题用目录名
    if state.title.is_empty() {
        if let Some(name) = root.file_name().and_then(|s| s.to_str()) {
            state.title = name.to_string();
        }
    }
    // 兜底：单章字数目标 —— state.yml 没有时从 bible/world-rules.md 的「单章建议字数」解析
    if state.chapter_target_words == 0 {
        state.chapter_target_words = parse_chapter_target_from_world_rules(root);
    }
    state
}

/// 从 bible/world-rules.md 解析「单章建议字数」行里的第一个数字（如「约 3000 字」→ 3000）。
/// 找不到或读不到返回 0。
fn parse_chapter_target_from_world_rules(root: &Path) -> u64 {
    let path = root.join("bible").join("world-rules.md");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    for line in text.lines() {
        if line.contains("单章建议字数") || line.contains("单章字数") {
            if let Some(n) = first_number_in(line) {
                return n;
            }
        }
    }
    0
}

/// 取一行里出现的第一个十进制整数（阿拉伯数字），没有则返回 None。
fn first_number_in(s: &str) -> Option<u64> {
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse().ok()
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
/// 若传了 base_fingerprint，写前做脏检查：磁盘 mtime/size 与指纹不一致 → 返回 conflict。
#[tauri::command]
fn write_chapter(args: WriteChapterArgs) -> Result<WriteChapterResult, String> {
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

    // 写前脏检查：仅当调用方给了 base 指纹时进行
    if let Some(fp) = args.base_fingerprint {
        if let Ok(meta) = fs::metadata(&path) {
            let disk_mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let disk_size = meta.len();
            if disk_mtime != fp.mtime_ms || disk_size != fp.size {
                return Ok(WriteChapterResult {
                    ok: false,
                    conflict: true,
                    mtime_ms: disk_mtime,
                    size: disk_size,
                });
            }
        }
    }

    fs::write(&path, args.content.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;

    // 写盘后的最新指纹（前端用来更新 base，避免下次保存误报冲突）
    let (mtime_ms, size) = match fs::metadata(&path) {
        Ok(m) => (
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            m.len(),
        ),
        Err(_) => (0, args.content.len() as u64),
    };
    Ok(WriteChapterResult {
        ok: true,
        conflict: false,
        mtime_ms,
        size,
    })
}

/// 取某文件的指纹（mtime 毫秒 + 字节数）；文件不存在返回 None。
fn file_fingerprint(path: &Path) -> Option<Fingerprint> {
    let meta = fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some(Fingerprint {
        mtime_ms,
        size: meta.len(),
    })
}

/// 文件快照里的一条记录（相对小说根路径 + mtime + 大小）。
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub path: String,
    pub mtime_ms: u64,
    pub size: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadChangesArgs {
    pub root: String,
    /// 上次快照；None 表示首次调用（只返回当前快照，changed=false）。
    pub last_seen: Option<Vec<FileStamp>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadChangesResult {
    pub changed: bool,
    pub stamps: Vec<FileStamp>,
}

/// 收集小说项目快照：state.yml + bible/（含 characters/）+ chapters/，只 stat 不读内容。
fn collect_stamps(root: &Path) -> Vec<FileStamp> {
    let mut out = Vec::new();
    if let Some(fp) = file_fingerprint(&root.join("state.yml")) {
        out.push(FileStamp { path: "state.yml".into(), mtime_ms: fp.mtime_ms, size: fp.size });
    }
    let bible = root.join("bible");
    if let Ok(entries) = fs::read_dir(&bible) {
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_file() && name.ends_with(".md") {
                if let Some(fp) = file_fingerprint(&p) {
                    out.push(FileStamp { path: format!("bible/{name}"), mtime_ms: fp.mtime_ms, size: fp.size });
                }
            } else if p.is_dir() && name == "characters" {
                if let Ok(chars) = fs::read_dir(&p) {
                    for c in chars.flatten() {
                        let cp = c.path();
                        let cn = c.file_name().to_string_lossy().to_string();
                        if cp.is_file() && cn.ends_with(".md") {
                            if let Some(fp) = file_fingerprint(&cp) {
                                out.push(FileStamp { path: format!("bible/characters/{cn}"), mtime_ms: fp.mtime_ms, size: fp.size });
                            }
                        }
                    }
                }
            }
        }
    }
    let chapters = root.join("chapters");
    if let Ok(entries) = fs::read_dir(&chapters) {
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_file() && name.ends_with(".md") {
                if let Some(fp) = file_fingerprint(&p) {
                    out.push(FileStamp { path: format!("chapters/{name}"), mtime_ms: fp.mtime_ms, size: fp.size });
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// 廉价变更检测：只 stat（不读正文），对比上次快照，返回是否有变化 + 当前快照。
/// 供前端 2s 轮询：DSH agent / 插件 / 任意编辑器改动了文件，桌面端据此自动刷新看板。
#[tauri::command]
fn read_changes(args: ReadChangesArgs) -> Result<ReadChangesResult, String> {
    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let stamps = collect_stamps(&novel_root);
    let changed = match args.last_seen {
        None => false, // 首次：只播种快照，不算变化
        Some(prev) => {
            if prev.len() != stamps.len() {
                true
            } else {
                prev.iter().zip(stamps.iter()).any(|(a, b)| {
                    a.path != b.path || a.mtime_ms != b.mtime_ms || a.size != b.size
                })
            }
        }
    };
    Ok(ReadChangesResult { changed, stamps })
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
    // 注意：Tauri app 从 Launch Services 启动，进程 PATH 是精简版（/usr/bin:/bin/...），
    // 不含 ~/.local/bin / /opt/homebrew/bin 等用户工具目录。所以显式 prepend 进去，
    // 让 `agt`（常装在 ~/.local/bin 或 brew 路径）也能被找到。
    let user_bin = std::env::var("HOME")
        .ok()
        .map(|h| format!("{h}/.local/bin"))
        .unwrap_or_default();
    let extra = [
        user_bin.as_str(),
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ]
    .iter()
    .filter(|p| !p.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(":");
    let current_path = std::env::var("PATH").unwrap_or_default();
    let mut agt_cmd = Command::new("agt");
    agt_cmd
        .arg("novel-init")
        .arg(&project_dir)
        .arg("--title")
        .arg(&args.title)
        .env(
            "PATH",
            if current_path.is_empty() {
                extra
            } else {
                format!("{extra}:{current_path}")
            },
        );
    let init_out = agt_cmd.output();
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
    let per_chapter: u64 = if total_chapters > 0 {
        target_words / u64::from(total_chapters)
    } else {
        0
    };
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
        per_ch = per_chapter,
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
         chapter_target_words: {per_chapter}   # 单章建议字数（编辑器达标 banner 用）\n\
         updated: \"{today}\"\n",
        title = args.title,
        era = args.era,
        pov_char_for_state = if args.pov_character.is_empty() {
            "（多重视角）".to_string()
        } else {
            args.pov_character.clone()
        },
        opening_hook = args.opening_hook,
        per_chapter = per_chapter,
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

// ===========================================================================
// 抉择点 / 剧情分支 / Story Spine
// ===========================================================================
//
// 数据结构（同 @ethanyoq/dsh-ai-novel-writer 的命名保持一致，避免将来要做
// 互操作时改名）：
//
// .ai-novel/
//   ├─ choice-points/cp-XXX.json        单次抉择（不重复）
//   ├─ branches/bp-XXX.json             单段分支剧情段（1~3 章汇回主干）
//   ├─ story-spine.json                 结构化剧情树
//   └─ choices.md                       决策索引（人类可读）
//
// "主干 + 关键岔路" 形态：每个抉择点 ≥1 个分支，分支结尾通常汇回主干。
// 形态选择见 strategy/04-product-shape 文档。
//
// ===========================================================================

const AI_NOVEL_DIR: &str = ".ai-novel";
const CHOICE_POINTS_DIR: &str = ".ai-novel/choice-points";
const BRANCHES_DIR: &str = ".ai-novel/branches";
const STORY_SPINE_JSON: &str = ".ai-novel/story-spine.json";
const CHOICES_INDEX_MD: &str = ".ai-novel/choices.md";

/// choice point 的"重量"等级，决定 AI 是否自动打断找人。
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ChoiceWeight {
    /// 不可回头的剧情转折。总是打断。
    Critical,
    /// 重要支线选择。>= major 总是打断。
    Major,
    /// 小决定，AI 自己选。
    Minor,
    /// 调味项，AI 自己选。
    Flavor,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    /// 1-2 句"如果选 X 接下来会怎样"，用户做决定时的预览。
    pub preview_hint: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRecord {
    pub by: String,           // "human" | "ai"
    pub option_id: String,
    pub decided_at: String,   // ISO 8601
    /// 自由文本备注（人类在决定时打的便签；AI 决定时可留评估原因）
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChoicePoint {
    pub id: String,
    pub weight: ChoiceWeight,
    /// 在哪个章节/分支之后。例："ch3" / "bp-001"。
    pub after_chapter: String,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    pub decided: Option<DecisionRecord>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChoicePointsView {
    pub root: String,
    pub ai_novel_dir_exists: bool,
    pub points: Vec<ChoicePoint>,
    /// 摘要：决定的总数 / 待定的总数
    pub decided_count: usize,
    pub pending_count: usize,
}

fn ai_novel_paths(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let base = root.join(AI_NOVEL_DIR);
    (
        base.join("choice-points"),
        base.join("branches"),
        base.join("story-spine.json"),
        base.clone(),
        base.join("choices.md"),
    )
}

fn next_cp_id(points_dir: &Path) -> String {
    // 找现有 cp-XXX.json 的最大编号
    let mut max_n: u32 = 0;
    if let Ok(rd) = fs::read_dir(points_dir) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if let Some(rest) = name.strip_prefix("cp-") {
                if let Some(rest) = rest.strip_suffix(".json") {
                    if let Ok(n) = rest.parse::<u32>() {
                        if n > max_n {
                            max_n = n;
                        }
                    }
                }
            }
        }
    }
    format!("cp-{:03}", max_n + 1)
}

fn cp_path(points_dir: &Path, id: &str) -> PathBuf {
    points_dir.join(format!("{id}.json"))
}

/// 读所有抉择点状态。
#[tauri::command]
fn read_choice_points(root: String) -> Result<ChoicePointsView, String> {
    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let (points_dir, _branches_dir, _spine, ai_novel_dir, _md_index) =
        ai_novel_paths(&novel_root);

    if !ai_novel_dir.is_dir() {
        return Ok(ChoicePointsView {
            root: novel_root.to_string_lossy().to_string(),
            ai_novel_dir_exists: false,
            points: Vec::new(),
            decided_count: 0,
            pending_count: 0,
        });
    }

    let mut points: Vec<ChoicePoint> = Vec::new();
    if let Ok(rd) = fs::read_dir(&points_dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                let text = match fs::read_to_string(&p) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                match serde_json::from_str::<ChoicePoint>(&text) {
                    Ok(cp) => points.push(cp),
                    Err(_) => continue,
                }
            }
        }
    }
    // 按 id 字典序（cp-001 ... cp-999 自然顺序）
    points.sort_by(|a, b| a.id.cmp(&b.id));

    let decided_count = points.iter().filter(|p| p.decided.is_some()).count();
    let pending_count = points.len() - decided_count;

    Ok(ChoicePointsView {
        root: novel_root.to_string_lossy().to_string(),
        ai_novel_dir_exists: true,
        points,
        decided_count,
        pending_count,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideChoiceArgs {
    pub root: String,
    pub point_id: String,
    pub option_id: String,
    pub by: String,           // "human" | "ai"
    pub note: Option<String>,
}

/// 用户（或 AI）做出决定。原子写，避免并发覆盖。
#[tauri::command]
fn decide_choice_point(args: DecideChoiceArgs) -> Result<DecisionRecord, String> {
    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let (points_dir, _, _, ai_novel_dir, _) = ai_novel_paths(&novel_root);
    fs::create_dir_all(&points_dir).map_err(|e| format!("mkdir {}: {e}", points_dir.display()))?;

    let path = cp_path(&points_dir, &args.point_id);
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("未找到抉择点 {}: {e}", path.display()))?;
    let mut cp: ChoicePoint = serde_json::from_str(&text)
        .map_err(|e| format!("抉择点 JSON 损坏：{e}"))?;

    if !cp.options.iter().any(|o| o.id == args.option_id) {
        return Err(format!(
            "选项 {} 不在该抉择点上。可用选项：{}",
            args.option_id,
            cp.options
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let now = chrono_like_today_compact();
    let decision = DecisionRecord {
        by: args.by,
        option_id: args.option_id.clone(),
        decided_at: format!("{now}T00:00:00Z"),
        note: args.note,
    };
    cp.decided = Some(decision.clone());

    let serialized = serde_json::to_string_pretty(&cp).map_err(|e| format!("serialize: {e}"))?;
    // 临时文件 + rename 原子写
    let tmp = path.with_file_name(format!("{}.tmp", args.point_id));
    fs::write(&tmp, serialized.as_bytes()).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;

    let _ = ai_novel_dir; // suppress unused
    Ok(decision)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChoicePointArgs {
    pub root: String,
    pub weight: String,         // "critical" | "major" | "minor" | "flavor"
    pub after_chapter: String,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    pub decided: Option<DecisionRecord>,
}

/// 创建一个新的抉择点。auto id（cp-001, cp-002...）。
#[tauri::command]
fn create_choice_point(args: CreateChoicePointArgs) -> Result<ChoicePoint, String> {
    if args.options.len() < 2 {
        return Err("至少需要 2 个选项".into());
    }
    if !args.options.iter().any(|o| o.id == "ai") {
        return Err("必须包含 id=\"ai\" 选项（让 AI 决定）".into());
    }
    let weight: ChoiceWeight = match args.weight.as_str() {
        "critical" => ChoiceWeight::Critical,
        "major" => ChoiceWeight::Major,
        "minor" => ChoiceWeight::Minor,
        "flavor" => ChoiceWeight::Flavor,
        other => return Err(format!("未知权重：{other}")),
    };

    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let (points_dir, _, _, _, _) = ai_novel_paths(&novel_root);
    fs::create_dir_all(&points_dir).map_err(|e| format!("mkdir {}: {e}", points_dir.display()))?;

    let id = next_cp_id(&points_dir);
    let cp = ChoicePoint {
        id: id.clone(),
        weight,
        after_chapter: args.after_chapter,
        prompt: args.prompt,
        options: args.options,
        decided: args.decided,
    };
    let path = cp_path(&points_dir, &id);
    let serialized = serde_json::to_string_pretty(&cp).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, serialized.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(cp)
}

/// 在项目里塞入 3 个示例抉择点（演示 / 调试用）。
/// 不会覆盖已有 cp-XXX.json；只补当前没有的。
#[tauri::command]
fn seed_demo_choice_points(root: String) -> Result<usize, String> {
    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let (points_dir, _, _, ai_novel_dir, _) = ai_novel_paths(&novel_root);
    fs::create_dir_all(&points_dir).map_err(|e| format!("mkdir {}: {e}", points_dir.display()))?;
    fs::create_dir_all(&ai_novel_dir).map_err(|e| format!("mkdir {}: {e}", ai_novel_dir.display()))?;

    // 演示数据按"用户最近讲过的剧情假设"塑造：玄幻长篇，cp-001 = 赦免叛徒
    // 这恰好呼应你刚才提到的设定。"递归嵌入"到我前面已经建好的 `/Users/zhouluyong/Documents/我的小说`
    // —— 让 demo 内容有意义。
    let demos: Vec<(&str, &str, &str, &str, Vec<(&str, &str, &str)>)> = vec![
        (
            "critical",
            "ch3",
            "你抓到叛徒萧承。他曾是挚友，叛逃去投靠了北境魔宗。",
            "cp-001",
            vec![
                ("a", "宽恕", "主角动摇，最终释放；萧承重回阵营内化矛盾"),
                ("b", "流放", "永不杀、不再见；萧承三月后于边塞再现"),
                ("c", "处决", "萧承死；伏笔 FX02「北境魔宗派系」自动销账"),
                ("ai", "让 AI 决定", "AI 评估三种走向的剧情张力并选最合适"),
            ],
        ),
        (
            "major",
            "ch5",
            "主角得知圣女被困在幽魂塔。塔中只有一条生路，主角只需自己进入。",
            "cp-002",
            vec![
                ("a", "独自前往", "主角独闯；三成几率阵亡。剧情集中，但弱化女主戏"),
                ("b", "招募萧承同去", "二人成行；提供萧承的救赎机会，加深羁绊"),
                ("c", "按兵不动", "圣女有她自己的命运；推进其它支线"),
                ("ai", "让 AI 决定", "AI 评估走向，选择剧情张力更大的一种"),
            ],
        ),
        (
            "minor",
            "ch6",
            "夜宿荒村。村中老妪煮了一锅来路不明的汤，请主角一行喝。",
            "cp-003",
            vec![
                ("a", "喝", "有毒的设定；解锁一段幻象伏笔"),
                ("b", "拒绝", "保持谨慎；老妪失望离去"),
                ("c", "让萧承先尝", "利用探子能力，萧承可识破毒物"),
                ("ai", "让 AI 决定", "这是 minor 等级；AI 可基于故事调性自选"),
            ],
        ),
    ];

    let mut added = 0usize;
    for (weight_str, after, prompt, _id_preview, options) in demos {
        let id = next_cp_id(&points_dir);
        let opts: Vec<ChoiceOption> = options
            .into_iter()
            .map(|(o, l, h)| ChoiceOption {
                id: o.to_string(),
                label: l.to_string(),
                preview_hint: h.to_string(),
            })
            .collect();
        let weight_enum: ChoiceWeight = match weight_str {
            "critical" => ChoiceWeight::Critical,
            "major" => ChoiceWeight::Major,
            "minor" => ChoiceWeight::Minor,
            "flavor" => ChoiceWeight::Flavor,
            _ => ChoiceWeight::Major,
        };
        // 第一个 demo 标记决定 = 人类 + 流放（呼应用户"我都要选 b"的偏好）
        let decided = if weight_str == "critical" {
            Some(DecisionRecord {
                by: "human".to_string(),
                option_id: "b".to_string(),
                decided_at: format!("{}T00:00:00Z", chrono_like_today_compact()),
                note: Some("demo 决定：流放；保留萧承的复活可能性".to_string()),
            })
        } else {
            None
        };
        let cp = ChoicePoint {
            id,
            weight: weight_enum,
            after_chapter: after.to_string(),
            prompt: prompt.to_string(),
            options: opts,
            decided,
        };
        let path = cp_path(&points_dir, &cp.id);
        let serialized = serde_json::to_string_pretty(&cp).map_err(|e| format!("serialize: {e}"))?;
        fs::write(&path, serialized.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
        added += 1;
    }

    Ok(added)
}

/// Compact YYYY-MM-DD (use chrono_like_today but stripped fmt)
fn chrono_like_today_compact() -> String {
    chrono_like_today()
}

// ===========================================================================
// 剧情树（Story Spine）— 章节 + 抉择点的结构化视图
// ===========================================================================

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SpineNode {
    /// "chapter" | "choice" | "branch"
    pub kind: String,
    pub id: String,
    pub title: String,
    pub weight: Option<String>,
    pub decided: Option<String>,
    pub chars: Option<u64>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorySpine {
    pub ok: bool,
    pub root: String,
    /// 主线节点（章节 + 抉择点按序）
    pub nodes: Vec<SpineNode>,
    /// 各抉择点下的备选分支（id 到列表）
    pub branches: Vec<(String, Vec<SpineNode>)>,
}

/// 读取剧情树：主线 = chapters/*.md + choice-points/cp-*.json 按序合并；
/// 备选分支 = 每个抉择点的非选中选项。
#[tauri::command]
fn read_story_spine(root: String) -> Result<StorySpine, String> {
    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;

    // 章节（已存在的文件）
    let mut chapters = list_chapters(&novel_root).map_err(err_to_string)?;
    chapters.sort_by(|a, b| natural_cmp(&a.file, &b.file));

    // 抉择点
    let (points_dir, _, _, _, _) = ai_novel_paths(&novel_root);
    let mut points: Vec<ChoicePoint> = Vec::new();
    if points_dir.is_dir() {
        if let Ok(rd) = fs::read_dir(&points_dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(text) = fs::read_to_string(&p) {
                        if let Ok(cp) = serde_json::from_str::<ChoicePoint>(&text) {
                            points.push(cp);
                        }
                    }
                }
            }
        }
    }
    points.sort_by(|a, b| a.id.cmp(&b.id));

    // 合并为主线：按 after_chapter 把抉择点插到对应章节后
    let mut nodes: Vec<SpineNode> = Vec::new();
    for ch in &chapters {
        nodes.push(SpineNode {
            kind: "chapter".into(),
            id: ch.file.clone(),
            title: ch.title.clone(),
            weight: None,
            decided: None,
            chars: Some(ch.chars),
        });
        // 该章节之后的抉择点
        for cp in &points {
            if cp.after_chapter == ch.file {
                nodes.push(SpineNode {
                    kind: "choice".into(),
                    id: cp.id.clone(),
                    title: cp.prompt.clone(),
                    weight: Some(format!("{:?}", cp.weight).to_ascii_lowercase()),
                    decided: cp.decided.as_ref().map(|d| d.option_id.clone()),
                    chars: None,
                });
            }
        }
    }
    // 没有任何抉择点明确挂在某章后 → 追加到末尾
    let known_chapters: Vec<String> = chapters.iter().map(|c| c.file.clone()).collect();
    for cp in &points {
        if !known_chapters.contains(&cp.after_chapter) {
            nodes.push(SpineNode {
                kind: "choice".into(),
                id: cp.id.clone(),
                title: cp.prompt.clone(),
                weight: Some(format!("{:?}", cp.weight).to_ascii_lowercase()),
                decided: cp.decided.as_ref().map(|d| d.option_id.clone()),
                chars: None,
            });
        }
    }

    // 备选分支：每个抉择点的非选中选项作为潜在分支
    let mut branches: Vec<(String, Vec<SpineNode>)> = Vec::new();
    for cp in &points {
        let chosen = cp.decided.as_ref().map(|d| d.option_id.as_str()).unwrap_or("");
        let alts: Vec<SpineNode> = cp
            .options
            .iter()
            .filter(|o| o.id != chosen)
            .map(|o| SpineNode {
                kind: "branch".into(),
                id: format!("{}/{}", cp.id, o.id),
                title: o.label.clone(),
                weight: None,
                decided: None,
                chars: None,
            })
            .collect();
        branches.push((cp.id.clone(), alts));
    }

    Ok(StorySpine {
        ok: true,
        root: novel_root.to_string_lossy().to_string(),
        nodes,
        branches,
    })
}

// ===========================================================================
// DSH agent 集成 — AI 写章节
// ===========================================================================
//
// 通过 dsh web 的 JSON-RPC over HTTP 驱动 agent：
//   1. session.create(cwd=小说目录, agentPreset="standard")
//   2. session.prompt(queue, 写作指令)
//   3. 轮询 session.history 直到 turn/end
//   4. 提取 assistant 最终文本
//
// 指令协议：我们给 agent 的 system 提示里约定：
//   - 正常写完一章 → 直接输出章节 markdown 全文
//   - 遇到关键抉择 → 输出 `@@CHOICE@@ {json} @@END@@` 后停住
//     抉择 json 形如 { "prompt": "...", "options": [ {id,label,previewHint}, ... ] }
//
// ===========================================================================

// ---------------------------------------------------------------------------
// DSH Web 登录（桌面端驱动 agent 前需要先登录拿 session cookie）
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshLoginArgs {
    pub username: String,
    pub password: String,
    /// TOTP 二步验证码（有 TOTP 的账号第二步才填）
    pub code: Option<String>,
    /// 第一步登录返回的 mfaToken（有 TOTP 时第二步回传）
    pub mfa_token: Option<String>,
    pub port: Option<u16>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshLoginResult {
    pub ok: bool,
    pub mfa_required: bool,
    pub mfa_token: Option<String>,
    pub message: String,
}

/// 登录 DSH Web。无 TOTP 的账号一次成功；有 TOTP 的账号先返回 mfaRequired + mfaToken，
/// 前端再让用户输 code 调第二次。
#[tauri::command]
fn dsh_login(args: DshLoginArgs) -> Result<DshLoginResult, String> {
    let port = args.port.unwrap_or(dsh_client::default_port());
    if !dsh_client::ping(port) {
        return Ok(DshLoginResult {
            ok: false,
            mfa_required: false,
            mfa_token: None,
            message: format!("DSH Web 未运行（127.0.0.1:{port}）"),
        });
    }
    let out = dsh_client::login(
        &args.username,
        &args.password,
        args.code.as_deref(),
        args.mfa_token.as_deref(),
        port,
    );
    Ok(DshLoginResult {
        ok: out.ok,
        mfa_required: out.mfa_required,
        mfa_token: out.mfa_token,
        message: out.message,
    })
}

/// 登出：清除已保存的会话 cookie。
#[tauri::command]
fn dsh_logout() -> Result<bool, String> {
    dsh_client::clear_cookie();
    Ok(true)
}

/// 是否已有保存的会话 cookie（粗略判断登录态）。
#[tauri::command]
fn dsh_login_status() -> Result<bool, String> {
    Ok(dsh_client::load_cookie().is_some())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiWriteChapterArgs {
    pub root: String,
    /// 附加指令（可选）：例如 "写第 3 章，聚焦林楚发现叛徒"。
    pub instruction: Option<String>,
    /// 会话 id（可选）：传了就复用，不传每次新建。
    pub session_id: Option<String>,
    /// 可选：等待超时（秒），默认 180。
    pub timeout_secs: Option<u64>,
    /// 可选：web 端口，默认 3080。
    pub port: Option<u16>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiWriteChapterResult {
    pub ok: bool,
    /// agent 最终输出的完整文本（可能包含 @@CHOICE@@ 标记）
    pub text: String,
    /// 若 agent 要求抉择，这里是非 None 的抉择 JSON
    pub choice_request: Option<Value>,
    /// 用于续写的会话 id（保持上下文）
    pub session_id: Option<String>,
    /// 是否已把文本写入 chapters/<file>
    pub saved_to: Option<String>,
    pub error: Option<String>,
}

/// 构建给 agent 的写作指令（把小说项目上下文 + 用户指令拼一起）。
fn build_writing_prompt(root: &Path, instruction: &str) -> Result<String, String> {
    let state_text = fs::read_to_string(root.join("state.yml")).unwrap_or_default();
    let bible_dir = root.join("bible");
    let mut bible_summary = String::new();
    if let Ok(entries) = fs::read_dir(&bible_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") {
                let p = e.path();
                if let Ok(text) = fs::read_to_string(&p) {
                    let head: String = text.chars().take(600).collect();
                    bible_summary.push_str(&format!("\n### {name}\n{head}\n"));
                }
            }
            if e.path().is_dir() && name == "characters" {
                if let Ok(chars) = fs::read_dir(e.path()) {
                    for c in chars.flatten() {
                        let cname = c.file_name().to_string_lossy().to_string();
                        if cname.ends_with(".md") {
                            if let Ok(text) = fs::read_to_string(c.path()) {
                                let head: String = text.chars().take(300).collect();
                                bible_summary.push_str(&format!("\n#### 角色 {cname}\n{head}\n"));
                            }
                        }
                    }
                }
            }
        }
    }

    let chapters_dir = root.join("chapters");
    let mut chapter_list = String::new();
    if let Ok(entries) = fs::read_dir(&chapters_dir) {
        let mut files: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|f| f.ends_with(".md"))
            .collect();
        files.sort();
        for f in files {
            chapter_list.push_str(&format!("- {f}\n"));
        }
    }

    Ok(format!(
        "你是一位小说家，正在为小说《{title}》创作。\n\
         工作目录：{cwd}\n\n\
         ## 项目状态（state.yml）\n\
         ```\n{state}\n```\n\n\
         ## 世界圣经摘要（bible/）\n\
         {bible}\n\n\
         ## 已有章节列表\n\
         {chapters}\n\n\
         ## 你的任务\n\
         {instruction}\n\n\
         ## 输出规则（重要）\n\
         - 直接输出新章节的 markdown 全文（以 # 开头的一级标题作为章节名）。\n\
         - 不要修改任何 bible/ 文件、不要创建额外文件。\n\
         - 如果写作过程中遇到【影响主线方向的抉择点】——角色生死、阵营归属、重大立场选择等——\n\
           不要自己决定，也不要继续往下写；停止并输出下面这个固定格式：\n\
           \n\
           @@CHOICE@@ {{\"prompt\": \"<把抉择用一句话写清楚>\", \"options\": [{{\"id\": \"a\", \"label\": \"<选项A>\", \"previewHint\": \"<选了A之后1-2句走向预览>\"}}, {{\"id\": \"b\", \"label\": \"<选项B>\", \"previewHint\": \"<选了B之后1-2句走向预览>\"}}, {{\"id\": \"c\", \"label\": \"<选项C>\", \"previewHint\": \"<选了C之后1-2句走向预览>\"}}, {{\"id\": \"ai\", \"label\": \"让 AI 决定\", \"previewHint\": \"AI 评估剧情张力后选择\"}}]}} @@END@@\n\
           \n\
           - 抉择的权重分级：critical（重大转折）永远停；major（重要支线）永远停；\n\
             minor（小决定）可以自己定，但如果你拿不准也可以停下来问。\n\
           - 一旦决定继续，从上一个抉择点接续写作，不要重复已写内容。",
        title = read_state(root).title,
        cwd = root.display(),
        state = state_text,
        bible = bible_summary,
        chapters = chapter_list,
        instruction = instruction,
    ))
}

/// 驱动 DSH agent 写一章。阻塞直到 agent 回合结束（或超时）。
#[tauri::command]
fn ai_write_chapter(args: AiWriteChapterArgs) -> Result<AiWriteChapterResult, String> {
    use dsh_client as dsh;

    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let port = args.port.unwrap_or(dsh::default_port());
    let timeout = args.timeout_secs.unwrap_or(180);

    if !dsh::ping(port) {
        return Ok(AiWriteChapterResult {
            ok: false,
            error: Some(format!(
                "DSH Web 服务未运行（127.0.0.1:{port}）。请先在 dsh-cockpit 的 🖥️ Web UI 面板里启动，或运行 `dsh web`。"
            )),
            ..Default::default()
        });
    }

    let instruction = args.instruction.unwrap_or_else(|| {
        let state = read_state(&novel_root);
        let next = state.current_chapter;
        format!("写第 {next} 章（ch{next}.md）。基于 state.yml 的当前进度与圣经，写出一章约 2000-3500 字、剧情推进、结尾留钩子的正文。")
    });
    let prompt = build_writing_prompt(&novel_root, &instruction).map_err(|e| e.to_string())?;

    // 1. create session
    let sid = dsh::session_create(
        &novel_root.to_string_lossy(),
        args.session_id.as_deref(),
        Some("standard"),
        port,
    )
    .map_err(|e| format!("创建 DSH 会话失败：{e}"))?;

    // 2. prompt
    dsh::session_prompt(&sid, &prompt, "queue", port).map_err(|e| format!("提交指令失败：{e}"))?;

    // 3. wait
    let outcome = dsh::wait_for_assistant(&sid, port, timeout).map_err(|e| e.to_string())?;

    let mut result = AiWriteChapterResult {
        ok: true,
        text: outcome.text.clone(),
        choice_request: outcome.choice_request.clone(),
        session_id: Some(sid),
        saved_to: None,
        error: None,
    };

    // 4. 如果没有抉择点，尝试把章节写入 chapters/ch{N}.md（自动命名）
    if outcome.choice_request.is_none() && !outcome.text.trim().is_empty() {
        let cleaned = extract_markdown_body(&outcome.text);
        // 从正文标题推断章号（# 第一章 → 1；# 第三章 → 3；找不到用 current_chapter）
        let inferred = infer_chapter_number(&cleaned);
        let state = read_state(&novel_root);
        let next_num: u32 = if let Some(n) = inferred {
            n
        } else {
            state.current_chapter.trim().parse::<u32>().unwrap_or(0)
        };
        let file = format!("ch{:03}.md", next_num);
        let target = novel_root.join("chapters").join(&file);
        // 已存在则找下一个空位（防止覆盖）
        let file = if target.exists() {
            let mut n = next_num + 1;
            loop {
                let candidate = format!("ch{:03}.md", n);
                if !novel_root.join("chapters").join(&candidate).exists() {
                    break candidate;
                }
                n += 1;
            }
        } else {
            file
        };
        let target = novel_root.join("chapters").join(&file);
        if fs::write(&target, cleaned.as_bytes()).is_ok() {
            result.saved_to = Some(file);
            // 更新 state.yml 的 current_chapter
            let next = next_num + 1;
            let updated = update_state_current_chapter(&novel_root, next);
            let _ = updated;
        }
    }

    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReconcileBibleArgs {
    pub root: String,
    /// 章节文件名（如 ch001.md），可选
    pub chapter_file: Option<String>,
    /// 用户刚做的决定（可选）："{point}: 决定选 X —— 备注"
    pub decision: Option<String>,
    pub session_id: Option<String>,
    pub timeout_secs: Option<u64>,
    pub port: Option<u16>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiReconcileBibleResult {
    pub ok: bool,
    pub text: String,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

/// 让 AI 读完最近一章 + 现有 bible，自动更新：
///   - bible/timeline.md（追加章节锚点）
///   - bible/foreshadowing.md（登记新伏笔 / 销账已收）
///   - 涉及的角色档案（本章变化记录）
///   - state.yml（foreshadowing_open 等）
/// 这是"每章收尾三件事"的 AI 机械化版本。
#[tauri::command]
fn ai_reconcile_bible(args: AiReconcileBibleArgs) -> Result<AiReconcileBibleResult, String> {
    use dsh_client as dsh;

    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let port = args.port.unwrap_or(dsh::default_port());
    let timeout = args.timeout_secs.unwrap_or(180);

    if !dsh::ping(port) {
        return Ok(AiReconcileBibleResult {
            ok: false,
            error: Some(format!("DSH Web 服务未运行（127.0.0.1:{port}）。请先启动 dsh web。")),
            ..Default::default()
        });
    }

    // 读取最近一章内容
    let chapter_text = if let Some(file) = &args.chapter_file {
        fs::read_to_string(novel_root.join("chapters").join(file))
            .unwrap_or_default()
    } else {
        // 找 chapters/ 里最后一个
        let mut files: Vec<String> = fs::read_dir(novel_root.join("chapters"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|f| f.ends_with(".md"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        if let Some(last) = files.pop() {
            fs::read_to_string(novel_root.join("chapters").join(&last)).unwrap_or_default()
        } else {
            String::new()
        }
    };

    let timeline = fs::read_to_string(novel_root.join("bible").join("timeline.md")).unwrap_or_default();
    let foreshadowing = fs::read_to_string(novel_root.join("bible").join("foreshadowing.md")).unwrap_or_default();
    let decision_note = args.decision.clone().unwrap_or_default();

    let prompt = format!(
        "你是小说连续性守门员。读下面的章节正文与现有 bible 摘录，产出【收尾三件事】的**结构化增量**。\n\
         \n\
         ## 铁律（违反即失败）\n\
         - 你的**整个回复**必须是一个合法 JSON 对象：第一个字符是 {{，最后一个字符是 }}。\n\
         - 绝对不要：markdown 围栏（```）、```json 标记、前言、解释、评论。\n\
         - 不要重写整个文件 —— 只给「追加/替换」的最小增量。\n\
         - 绝对不要输出章节正文或小说草稿。\n\
         \n\
         ## 输出格式（严格 JSON，只输出这个）\n\
         {{\n\
           \"timelineRows\": [ \"| ch1 | 世界历 47 年秋 | 17 | 王城 | 李四捡到怀表 | 李四 |\" ],\n\
           \"foreshadowingRows\": [ \"| F001 | ch1 | 怀表停在3:47 | 杂货铺 | 埋设 | —\" ],\n\
           \"foreshadowingResolve\": [ {{ \"id\": \"F001\", \"chapter\": \"ch1\", \"way\": \"如何回收\" }} ],\n\
           \"characterUpdates\": [ {{\"file\": \"characters/李四\", \"note\": \"捡到怀表，开始怀疑时间\" }} ]\n\
         }}\n\
         - timelineRows：追加到 timeline.md「章节锚点」表（在（追加）占位行前）\n\
         - foreshadowingRows：追加到 foreshadowing.md 表格（在（追加）行前）\n\
         - foreshadowingResolve：把对应 ID 的状态从「待收」改为「已收（chN）」\n\
         - characterUpdates：在角色档案的「本章变化记录」追加一行\n\
         - 没有的数组留空 []\n\
         - 回复里不要有任何 JSON 以外的字符。\n\
         \n\
         ## 用户刚做的决定（若有，影响剧情）\n\
         {decision}\n\
         \n\
         ## 章节正文（最近一章）\n\
         ```markdown\n{chapter}\n```\n\
         \n\
         ## 现有 timeline.md（只摘录表格部分）\n\
         ```\n{timeline}\n```\n\
         \n\
         ## 现有 foreshadowing.md\n\
         ```\n{foreshadowing}\n```",
        decision = decision_note,
        chapter = if chapter_text.chars().count() > 3500 {
            chapter_text.chars().take(3500).collect::<String>()
        } else {
            chapter_text.clone()
        },
        timeline = if timeline.chars().count() > 2000 { timeline.chars().take(2000).collect::<String>() } else { timeline },
        foreshadowing = if foreshadowing.chars().count() > 2000 { foreshadowing.chars().take(2000).collect::<String>() } else { foreshadowing },
    );

    let sid = dsh::session_create(
        &novel_root.to_string_lossy(),
        args.session_id.as_deref(),
        Some("standard"),
        port,
    )
    .map_err(|e| format!("创建 DSH 会话失败：{e}"))?;

    dsh::session_prompt(&sid, &prompt, "queue", port).map_err(|e| format!("提交指令失败：{e}"))?;
    let outcome = dsh::wait_for_assistant(&sid, port, timeout).map_err(|e| e.to_string())?;
    let text = outcome.text.clone();

    // 解析 JSON（容忍被 ```json 围栏包裹）
    let json_str = extract_json_object(&text);
    let delta: Value = match json_str.as_deref().and_then(|s| serde_json::from_str(s).ok()) {
        Some(v) => v,
        None => {
            return Ok(AiReconcileBibleResult {
                ok: false,
                text,
                session_id: Some(sid),
                error: Some("agent 没有输出合法 JSON 增量（reconcile 失败）".into()),
            });
        }
    };

    // 应用 delta 到 bible 文件
    let mut applied = 0usize;
    let bible_dir = novel_root.join("bible");

    // timeline rows
    if let Some(rows) = delta.get("timelineRows").and_then(|r| r.as_array()) {
        if !rows.is_empty() {
            let tl_path = bible_dir.join("timeline.md");
            if let Ok(existing) = fs::read_to_string(&tl_path) {
                let rows_text: Vec<String> = rows
                    .iter()
                    .filter_map(|r| r.as_str().map(|s| s.to_string()))
                    .collect();
                let updated = insert_rows_before_append(&existing, &rows_text);
                if fs::write(&tl_path, updated.as_bytes()).is_ok() {
                    applied += rows.len();
                }
            }
        }
    }

    // foreshadowing rows
    if let Some(rows) = delta.get("foreshadowingRows").and_then(|r| r.as_array()) {
        if !rows.is_empty() {
            let fs_path = bible_dir.join("foreshadowing.md");
            if let Ok(existing) = fs::read_to_string(&fs_path) {
                let rows_text: Vec<String> = rows
                    .iter()
                    .filter_map(|r| r.as_str().map(|s| s.to_string()))
                    .collect();
                let updated = insert_rows_before_append(&existing, &rows_text);
                if fs::write(&fs_path, updated.as_bytes()).is_ok() {
                    applied += rows.len();
                }
            }
        }
    }

    // foreshadowing resolve（状态列：待收 → 已收(chN)）
    if let Some(resolves) = delta.get("foreshadowingResolve").and_then(|r| r.as_array()) {
        for res in resolves {
            let id = res.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let ch = res.get("chapter").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() || ch.is_empty() {
                continue;
            }
            let fs_path = bible_dir.join("foreshadowing.md");
            if let Ok(existing) = fs::read_to_string(&fs_path) {
                let updated = resolve_foreshadow_row(&existing, id, ch);
                if fs::write(&fs_path, updated.as_bytes()).is_ok() {
                    applied += 1;
                }
            }
        }
    }

    // character updates
    if let Some(updates) = delta.get("characterUpdates").and_then(|r| r.as_array()) {
        for upd in updates {
            let file = upd.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let note = upd.get("note").and_then(|v| v.as_str()).unwrap_or("");
            if file.is_empty() || note.is_empty() {
                continue;
            }
            let char_path = if file.starts_with("characters/") {
                bible_dir.join(format!("{file}.md"))
            } else {
                bible_dir.join("characters").join(format!("{file}.md"))
            };
            if let Ok(existing) = fs::read_to_string(&char_path) {
                let updated = append_character_change_note(&existing, note);
                if fs::write(&char_path, updated.as_bytes()).is_ok() {
                    applied += 1;
                }
            }
        }
    }

    Ok(AiReconcileBibleResult {
        ok: true,
        text,
        session_id: Some(sid),
        error: if applied == 0 { Some("delta 为空或全部未应用".into()) } else { None },
    })
}

/// 从文本里提取第一个 JSON 对象（容忍 ```json 围栏和前后说明文字）。
fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    // 找匹配的右花括号（简单计数，忽略字符串内 {} —— 够用）
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in text[start..].char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// 在表格的「（追加）」占位行前插入若干行。
fn insert_rows_before_append(existing: &str, rows: &[String]) -> String {
    let lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    let mut insert_at: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.contains("（追加）") || line.contains("(追加)") {
            insert_at = Some(i);
            break;
        }
    }
    match insert_at {
        Some(idx) => {
            let mut out = Vec::with_capacity(lines.len() + rows.len());
            out.extend_from_slice(&lines[..idx]);
            for r in rows {
                out.push(r.clone());
            }
            out.extend_from_slice(&lines[idx..]);
            out.join("\n")
        }
        None => {
            let mut out = existing.trim_end().to_string();
            for r in rows {
                out.push('\n');
                out.push_str(r);
            }
            out.push('\n');
            out
        }
    }
}

/// 把 foreshadowing.md 里某行的「待收」改为「已收（chN）」。
fn resolve_foreshadow_row(existing: &str, id: &str, chapter: &str) -> String {
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    for line in lines.iter_mut() {
        if line.trim_start().starts_with('|') && line.contains(id) {
            // 找「状态」列：| ID | 埋设章节 | 物件 | 埋点 | 状态 | 回收方式 |
            let parts: Vec<&str> = line.split('|').map(|p| p.trim()).collect();
            if parts.len() >= 7 && parts[5] == "待收" {
                // 重建：| a | b | c | d | 已收（chN） | 见本章正文 |
                let mut body: Vec<String> = Vec::with_capacity(6);
                for (i, p) in parts.iter().enumerate().skip(1).take(6) {
                    let cell = match i {
                        5 => format!("已收（{chapter}）"),
                        6 if p.is_empty() || *p == "—" => "见本章正文".to_string(),
                        _ => p.to_string(),
                    };
                    body.push(cell);
                }
                *line = format!("| {} |", body.join(" | "));
                break;
            }
        }
    }
    lines.join("\n")
}

/// 在角色档案的「本章变化记录」段追加一行。
fn append_character_change_note(existing: &str, note: &str) -> String {
    // 找「## 当前状态」下的「- 本章变化记录（N 章：…）」行，或文件末尾追加
    if let Some(pos) = existing.find("本章变化记录") {
        // 在该行后追加新一行
        let line_end = existing[pos..].find('\n').map(|i| pos + i).unwrap_or(existing.len());
        let mut out = existing.to_string();
        out.insert_str(line_end + 1, &format!("- 本章变化记录（续）：{note}\n"));
        out
    } else {
        let mut out = existing.trim_end().to_string();
        out.push_str(&format!("\n\n## 当前状态（追加）\n- 本章变化记录：{note}\n"));
        out
    }
}

// ===========================================================================
// AI 审核员 — 章节一致性审核（A/B/C/D/P 五类）
// ===========================================================================
//
// 让一个 agent 以"连续性守门员"身份读章节 + bible，产出结构化审核报告：
//   A = 严重冲突（与已写事实矛盾、时间倒流、已死角色复活）
//   B = 性格漂移（角色行为与档案不符）
//   C = 信息差漏洞（POV 角色知道了他不该知道的）
//   D = 伏笔推进（该埋没埋 / 该收没收 / 重复埋设）
//   P = 节奏问题（头重脚轻、视角漂移、对白失衡）
//
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewChapterArgs {
    pub root: String,
    /// 章节文件名（ch001.md 等）；缺省用最近一章
    pub chapter_file: Option<String>,
    pub session_id: Option<String>,
    pub timeout_secs: Option<u64>,
    pub port: Option<u16>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReviewIssue {
    pub severity: String,      // "critical" | "major" | "minor" | "info"
    pub location: String,      // 大致位置（章内第几段 / 原文引用）
    pub issue: String,         // 问题描述
    pub suggestion: String,    // 修订建议
}

#[derive(Serialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCategory {
    pub label: String,
    pub issues: Vec<ReviewIssue>,
}

#[derive(Serialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    pub ok: bool,
    pub chapter_file: String,
    pub summary: String,
    /// verdict: "pass"（无重大冲突）| "revise"（需要修订）
    pub verdict: String,
    pub categories: std::collections::HashMap<String, ReviewCategory>,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

/// 让 AI 审核最近一章（或指定章节）。
#[tauri::command]
fn ai_review_chapter(args: AiReviewChapterArgs) -> Result<ReviewReport, String> {
    use dsh_client as dsh;

    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let port = args.port.unwrap_or(dsh::default_port());
    let timeout = args.timeout_secs.unwrap_or(240);

    if !dsh::ping(port) {
        return Ok(ReviewReport {
            ok: false,
            error: Some(format!("DSH Web 服务未运行（127.0.0.1:{port}）。请先启动 dsh web。")),
            ..Default::default()
        });
    }

    // 确定章节文件
    let chapter_file = if let Some(f) = &args.chapter_file {
        f.clone()
    } else {
        let mut files: Vec<String> = fs::read_dir(novel_root.join("chapters"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|f| f.ends_with(".md"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files.pop().ok_or_else(|| "chapters/ 里没有章节可审".to_string())?
    };
    let chapter_text = fs::read_to_string(novel_root.join("chapters").join(&chapter_file))
        .map_err(|e| format!("读章节失败: {e}"))?;

    // 收集 bible 上下文（精简）
    let mut bible_ctx = String::new();
    let bible_dir = novel_root.join("bible");
    if let Ok(entries) = fs::read_dir(&bible_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") && name != "world-rules.md" {
                if let Ok(text) = fs::read_to_string(e.path()) {
                    let head: String = text.chars().take(1200).collect();
                    bible_ctx.push_str(&format!("\n### {name}\n{head}\n"));
                }
            }
            if e.path().is_dir() && name == "characters" {
                if let Ok(chars) = fs::read_dir(e.path()) {
                    for c in chars.flatten() {
                        let cname = c.file_name().to_string_lossy().to_string();
                        if cname.ends_with(".md") {
                            if let Ok(text) = fs::read_to_string(c.path()) {
                                let head: String = text.chars().take(700).collect();
                                bible_ctx.push_str(&format!("\n#### 角色 {cname}\n{head}\n"));
                            }
                        }
                    }
                }
            }
        }
    }

    let prompt = format!(
        "你是小说的连续性审核员（Continuity Auditor）。审核下面这章《{title}》的正文，对照现有世界圣经，找出连续性 / 信息差 / 伏笔 / 节奏问题。\n\
         \n\
         ## 五类问题（严格按此分类）\n\
         - A 严重冲突：与已写事实矛盾（时间倒流、已死角色复活、地点/阵营错乱、年龄/季节漂移）\n\
         - B 性格漂移：角色行为/说话方式与其档案不符\n\
         - C 信息差漏洞：POV 角色知道了他不该知道的信息（超出其已知信息段）\n\
         - D 伏笔推进：该埋的没埋、该收的没收、重复埋设同一伏笔\n\
         - P 节奏问题：章节头重脚轻、视角漂移、对白失衡、章末钩子缺失\n\
         \n\
         ## 输出格式（严格 JSON，不要任何其他文字/围栏）\n\
         {{\n\
           \"summary\": \"一句话总结本章质量\",\n\
           \"verdict\": \"pass\" 或 \"revise\",\n\
           \"categories\": {{\n\
             \"A\": {{\"label\": \"严重冲突\", \"issues\": [{{\"severity\": \"critical|major|minor|info\", \"location\": \"<章内位置>\", \"issue\": \"<问题>\", \"suggestion\": \"<修订建议>\"}}]}},\n\
             \"B\": {{\"label\": \"性格漂移\", \"issues\": [...]}},\n\
             \"C\": {{\"label\": \"信息差漏洞\", \"issues\": [...]}},\n\
             \"D\": {{\"label\": \"伏笔推进\", \"issues\": [...]}},\n\
             \"P\": {{\"label\": \"节奏问题\", \"issues\": [...]}}\n\
           }}\n\
         }}\n\
         - 某类没问题就 issues: []。\n\
         - severity 建议：致命逻辑错 critical；明显矛盾 major；可打磨 minor；提示 info。\n\
         - 尽量具体（引用原文短句），suggestion 给出可操作的改法。\n\
         \n\
         ## 章节正文\n\
         ```markdown\n{chapter}\n```\n\
         \n\
         ## 世界圣经摘要\n\
         {bible}\n\
         \n\
         ## 上一章钩子（state.yml next_hook，若有）\n\
         {next_hook}",
        title = read_state(&novel_root).title,
        chapter = if chapter_text.chars().count() > 5000 {
            chapter_text.chars().take(5000).collect::<String>()
        } else {
            chapter_text.clone()
        },
        bible = if bible_ctx.chars().count() > 4000 {
            bible_ctx.chars().take(4000).collect::<String>()
        } else {
            bible_ctx
        },
        next_hook = read_state(&novel_root).next_hook,
    );

    let sid = dsh::session_create(
        &novel_root.to_string_lossy(),
        args.session_id.as_deref(),
        Some("standard"),
        port,
    )
    .map_err(|e| format!("创建 DSH 会话失败：{e}"))?;
    dsh::session_prompt(&sid, &prompt, "queue", port).map_err(|e| format!("提交指令失败：{e}"))?;
    let outcome = dsh::wait_for_assistant(&sid, port, timeout).map_err(|e| e.to_string())?;

    // 解析 JSON 报告
    let json_str = extract_json_object(&outcome.text);
    let parsed: Option<Value> = json_str.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let mut report = ReviewReport {
        ok: true,
        chapter_file,
        summary: String::new(),
        verdict: "revise".into(),
        categories: Default::default(),
        session_id: Some(sid.clone()),
        error: None,
    };

    match parsed {
        Some(v) => {
            report.summary = v.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string();
            report.verdict = v.get("verdict").and_then(|s| s.as_str()).unwrap_or("revise").to_string();
            if let Some(cats) = v.get("categories").and_then(|c| c.as_object()) {
                for (key, cat) in cats {
                    let label = cat.get("label").and_then(|l| l.as_str()).unwrap_or(key).to_string();
                    let mut issues = Vec::new();
                    if let Some(iss) = cat.get("issues").and_then(|i| i.as_array()) {
                        for it in iss {
                            issues.push(ReviewIssue {
                                severity: it.get("severity").and_then(|s| s.as_str()).unwrap_or("minor").to_string(),
                                location: it.get("location").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                                issue: it.get("issue").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                                suggestion: it.get("suggestion").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                            });
                        }
                    }
                    report.categories.insert(key.clone(), ReviewCategory { label, issues });
                }
            }
        }
        None => {
            report.error = Some("agent 未输出合法 JSON 审核报告".into());
        }
    }

    // 审核报告存档到 .ai-novel/reviews/<chapter>-<ts>.json（防丢、可追溯）
    if report.ok && report.error.is_none() {
        let reviews_dir = novel_root.join(AI_NOVEL_DIR).join("reviews");
        let _ = fs::create_dir_all(&reviews_dir);
        if let Ok(json) = serde_json::to_string_pretty(&report) {
            let ts = chrono_like_today_compact().replace('-', "");
            let fname = format!("{}-{}.json", report.chapter_file.replace(".md", ""), ts);
            let _ = fs::write(reviews_dir.join(fname), json.as_bytes());
        }
    }

    Ok(report)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviseChapterArgs {
    pub root: String,
    pub chapter_file: String,
    /// 审核报告的 categories（前端把已有报告透传回来），或让 agent 重读
    pub report_json: Option<String>,
    pub session_id: Option<String>,
    pub timeout_secs: Option<u64>,
    pub port: Option<u16>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiReviseChapterResult {
    pub ok: bool,
    pub revised_text: String,
    pub saved_to: Option<String>,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

/// 基于审核报告修订章节，生成 v2 并覆盖写回（保留原版为 .bak）。
#[tauri::command]
fn ai_revise_chapter(args: AiReviseChapterArgs) -> Result<AiReviseChapterResult, String> {
    use dsh_client as dsh;

    let start = PathBuf::from(&args.root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let port = args.port.unwrap_or(dsh::default_port());
    let timeout = args.timeout_secs.unwrap_or(240);

    if !dsh::ping(port) {
        return Ok(AiReviseChapterResult {
            ok: false,
            error: Some(format!("DSH Web 服务未运行（127.0.0.1:{port}）。请先启动 dsh web。")),
            ..Default::default()
        });
    }

    let chapter_path = novel_root.join("chapters").join(&args.chapter_file);
    let chapter_text = fs::read_to_string(&chapter_path)
        .map_err(|e| format!("读章节失败: {e}"))?;

    let report_text = args.report_json.clone().unwrap_or_default();

    let prompt = format!(
        "你是一位经验丰富的网文修订编辑。根据审核报告，修订下面的章节，生成修订版 v2。\n\
         \n\
         ## 修订原则\n\
         - 修复所有 A/B/C 类问题（严重冲突 / 性格漂移 / 信息差）；D/P 类酌情处理。\n\
         - 保持原章的叙事节奏、文风、伏笔走向；不要为了改而改。\n\
         - 只输出修订后的完整章节 markdown（以 # 开头），不要输出任何说明文字。\n\
         \n\
         ## 审核报告（JSON）\n\
         {report}\n\
         \n\
         ## 原章节正文\n\
         ```markdown\n{chapter}\n```",
        report = if report_text.chars().count() > 3000 { report_text.chars().take(3000).collect::<String>() } else { report_text },
        chapter = if chapter_text.chars().count() > 6000 { chapter_text.chars().take(6000).collect::<String>() } else { chapter_text.clone() },
    );

    let sid = dsh::session_create(
        &novel_root.to_string_lossy(),
        args.session_id.as_deref(),
        Some("standard"),
        port,
    )
    .map_err(|e| format!("创建 DSH 会话失败：{e}"))?;
    dsh::session_prompt(&sid, &prompt, "queue", port).map_err(|e| format!("提交指令失败：{e}"))?;
    let outcome = dsh::wait_for_assistant(&sid, port, timeout).map_err(|e| e.to_string())?;

    let revised = extract_markdown_body(&outcome.text);
    if revised.trim().is_empty() {
        return Ok(AiReviseChapterResult {
            ok: false,
            error: Some("agent 没有输出修订正文".into()),
            session_id: Some(sid),
            ..Default::default()
        });
    }

    // 备份原版 → .bak，然后覆盖写回
    let bak_path = chapter_path.with_extension("md.bak");
    let _ = fs::write(&bak_path, chapter_text.as_bytes());
    if fs::write(&chapter_path, revised.as_bytes()).is_err() {
        // 写失败尝试恢复
        let _ = fs::write(&chapter_path, chapter_text.as_bytes());
        return Ok(AiReviseChapterResult {
            ok: false,
            error: Some("写入修订版失败".into()),
            session_id: Some(sid),
            ..Default::default()
        });
    }

    Ok(AiReviseChapterResult {
        ok: true,
        revised_text: revised.clone(),
        saved_to: Some(args.chapter_file.clone()),
        session_id: Some(sid),
        error: None,
    })
}

// ===========================================================================
// 一键流水线：写 → 审 → 改 → 终稿
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFullPipelineArgs {
    pub root: String,
    /// 写作指令（可选，缺省按进度写下一章）
    pub instruction: Option<String>,
    /// 是否自动修订（verdict=revise 且 A/B/C 有重大问题时）；默认 true
    pub auto_revise: Option<bool>,
    /// 是否自动收尾（reconcile bible）；默认 true
    pub auto_reconcile: Option<bool>,
    pub session_id: Option<String>,
    pub timeout_secs: Option<u64>,
    pub port: Option<u16>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiFullPipelineResult {
    pub ok: bool,
    /// 当前所处阶段：write_done / reviewed / revised / done / error
    pub stage: String,
    /// 写出的章节文件名
    pub chapter_file: Option<String>,
    /// 审核报告摘要
    pub review_summary: Option<String>,
    pub verdict: Option<String>,
    /// 终稿文本（若修订过则是 v2，否则 v1）
    pub final_text: Option<String>,
    /// reconcile 结果简述
    pub reconcile_note: Option<String>,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

/// 一键走完 写 → 审 → 改 → 收尾 的完整循环。
/// 每步结束返回 stage，前端可据此刷新 UI。
#[tauri::command]
fn ai_full_pipeline(args: AiFullPipelineArgs) -> Result<AiFullPipelineResult, String> {
    let port = args.port.unwrap_or(dsh_client::default_port());
    let timeout = args.timeout_secs.unwrap_or(600);
    let mut session_id = args.session_id.clone();

    // 1. 写
    let write_res = ai_write_chapter(AiWriteChapterArgs {
        root: args.root.clone(),
        instruction: args.instruction.clone(),
        session_id: session_id.clone(),
        timeout_secs: Some(timeout.min(300)),
        port: Some(port),
    })?;

    if let Some(sid) = write_res.session_id.as_ref() {
        session_id = Some(sid.clone());
    }
    let chapter_file = match write_res.saved_to {
        Some(f) => f,
        None => {
            // 可能有抉择点暂停 —— 此时不能继续流水线
            return Ok(AiFullPipelineResult {
                ok: false,
                stage: if write_res.choice_request.is_some() { "choice_pending" } else { "error" }.into(),
                chapter_file: None,
                review_summary: None,
                verdict: None,
                final_text: None,
                reconcile_note: None,
                session_id,
                error: if write_res.choice_request.is_some() {
                    Some("写作遇到抉择点，已暂停等待你决定（请到「AI 写章节」处理后再继续）".into())
                } else {
                    write_res.error.clone()
                },
            });
        }
    };

    // 2. 审
    let review_res = ai_review_chapter(AiReviewChapterArgs {
        root: args.root.clone(),
        chapter_file: Some(chapter_file.clone()),
        session_id: session_id.clone(),
        timeout_secs: Some(timeout.min(240)),
        port: Some(port),
    })?;
    if let Some(sid) = review_res.session_id.as_ref() {
        session_id = Some(sid.clone());
    }

    let mut final_text = write_res.text.clone();
    let verdict = review_res.verdict.clone();
    let review_summary = review_res.summary.clone();

    // 3. 改（若自动修订开启且 verdict=revise 且有 A/B/C 问题）
    let mut revised = false;
    let auto_revise = args.auto_revise.unwrap_or(true);
    if auto_revise && review_res.verdict == "revise" {
        let abc_count = review_res
            .categories
            .get("A")
            .map(|c| c.issues.len())
            .unwrap_or(0)
            + review_res
                .categories
                .get("B")
                .map(|c| c.issues.len())
                .unwrap_or(0)
            + review_res
                .categories
                .get("C")
                .map(|c| c.issues.len())
                .unwrap_or(0);
        if abc_count > 0 {
            let revise_res = ai_revise_chapter(AiReviseChapterArgs {
                root: args.root.clone(),
                chapter_file: chapter_file.clone(),
                report_json: Some(
                    serde_json::to_string(&review_res)
                        .unwrap_or_default(),
                ),
                session_id: session_id.clone(),
                timeout_secs: Some(timeout.min(240)),
                port: Some(port),
            })?;
            if let Some(sid) = revise_res.session_id.as_ref() {
                session_id = Some(sid.clone());
            }
            if revise_res.ok {
                final_text = revise_res.revised_text.clone();
                revised = true;
            }
        }
    }

    // 4. 收尾（reconcile bible）
    let mut reconcile_note = None;
    let auto_reconcile = args.auto_reconcile.unwrap_or(true);
    if auto_reconcile {
        let rec_res = ai_reconcile_bible(AiReconcileBibleArgs {
            root: args.root.clone(),
            chapter_file: Some(chapter_file.clone()),
            decision: None,
            session_id: session_id.clone(),
            timeout_secs: Some(timeout.min(180)),
            port: Some(port),
        })?;
        if let Some(sid) = rec_res.session_id.as_ref() {
            session_id = Some(sid.clone());
        }
        reconcile_note = Some(if rec_res.ok {
            rec_res.error.unwrap_or_else(|| "圣经已同步".into())
        } else {
            format!("reconcile 未完成：{}", rec_res.error.unwrap_or_default())
        });
    }

    // 5. git 提交（防丢）
    let _ = git_commit_novel(&PathBuf::from(&args.root), &chapter_file, revised);

    Ok(AiFullPipelineResult {
        ok: true,
        stage: if revised { "done_revised".into() } else { "done".into() },
        chapter_file: Some(chapter_file.clone()),
        review_summary: Some(review_summary),
        verdict: Some(verdict.clone()),
        final_text: Some(final_text),
        reconcile_note,
        session_id,
        error: None,
    })
}

/// 在小说项目里执行 git add + commit（防丢）。项目可能没 git init → 静默跳过。
fn git_commit_novel(root: &Path, chapter_file: &str, revised: bool) -> Result<(), String> {
    use std::process::Command;

    // 确认 .git 存在，否则跳过
    if !root.join(".git").exists() {
        return Ok(());
    }
    let action = if revised { "修订" } else { "新增" };
    let out = Command::new("git")
        .args(["-C", root.to_str().unwrap_or("."), "add", "-A"])
        .output()
        .map_err(|e| format!("git add 失败: {e}"))?;
    if !out.status.success() {
        return Ok(()); // add 失败不致命
    }
    let _ = Command::new("git")
        .args(["-C", root.to_str().unwrap_or("."), "commit", "-m", &format!("ch: {action} {chapter_file}")])
        .output();
    Ok(())
}

/// 手动/里程碑提交的结果（前端用来展示提交状态与 hash）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitResult {
    pub repo_exists: bool,
    pub committed: bool,
    pub message: String,
    pub hash: Option<String>,
    pub summary: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInitResult {
    pub ok: bool,
    pub repo_exists: bool,
    pub summary: String,
}

/// 在小说项目里做一次快照提交（git add -A + commit）。
/// - 项目没 .git → 返回 repo_exists=false（前端提示「初始化 Git」）
/// - 没有改动 → committed=false、summary=「没有需要提交的改动」
/// - 成功 → committed=true + 短 hash
#[tauri::command]
fn git_commit(root: String, message: Option<String>) -> Result<GitCommitResult, String> {
    use std::process::Command;

    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let dir = novel_root.to_string_lossy().to_string();
    let msg = message
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "快照".to_string());

    let no_repo = GitCommitResult {
        repo_exists: false,
        committed: false,
        message: msg.clone(),
        hash: None,
        summary: "项目还未初始化 Git，点击「初始化 Git」".into(),
    };
    if !novel_root.join(".git").exists() {
        return Ok(no_repo);
    }

    // 没有改动就不提交
    match Command::new("git").args(["-C", &dir, "status", "--porcelain"]).output() {
        Ok(o) if o.stdout.is_empty() => {
            return Ok(GitCommitResult {
                repo_exists: true,
                committed: false,
                message: msg.clone(),
                hash: None,
                summary: "没有需要提交的改动".into(),
            });
        }
        Ok(_) => {}
        Err(e) => {
            return Ok(GitCommitResult {
                repo_exists: true,
                committed: false,
                message: msg.clone(),
                hash: None,
                summary: format!("git status 失败：{e}"),
            });
        }
    }

    // add + commit
    let add = Command::new("git").args(["-C", &dir, "add", "-A"]).output();
    match add {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Ok(GitCommitResult {
                repo_exists: true,
                committed: false,
                message: msg.clone(),
                hash: None,
                summary: format!("git add 失败：{}", truncate_str(&String::from_utf8_lossy(&o.stderr), 200)),
            });
        }
        Err(e) => {
            return Ok(GitCommitResult {
                repo_exists: true,
                committed: false,
                message: msg.clone(),
                hash: None,
                summary: format!("git add 失败：{e}"),
            });
        }
    }

    match Command::new("git").args(["-C", &dir, "commit", "-m", &msg]).output() {
        Ok(o) if o.status.success() => {
            let hash = Command::new("git")
                .args(["-C", &dir, "rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .map(|h| String::from_utf8_lossy(&h.stdout).trim().to_string())
                .filter(|s| !s.is_empty());
            let short = hash.clone().unwrap_or_default();
            Ok(GitCommitResult {
                repo_exists: true,
                committed: true,
                message: msg.clone(),
                hash,
                summary: format!("已提交 {short}"),
            })
        }
        Ok(o) => Ok(GitCommitResult {
            repo_exists: true,
            committed: false,
            message: msg.clone(),
            hash: None,
            summary: format!(
                "提交失败：{}",
                truncate_str(&String::from_utf8_lossy(&o.stderr), 200)
            ),
        }),
        Err(e) => Ok(GitCommitResult {
            repo_exists: true,
            committed: false,
            message: msg.clone(),
            hash: None,
            summary: format!("提交失败：{e}"),
        }),
    }
}

/// 在小说项目根执行 `git init`（幂等：已存在则直接返回成功）。
#[tauri::command]
fn git_init(root: String) -> Result<GitInitResult, String> {
    use std::process::Command;

    let start = PathBuf::from(&root);
    let novel_root = find_novel_root(&start)
        .ok_or_else(|| format!("未找到小说项目（{}）", start.display()))?;
    let dir = novel_root.to_string_lossy().to_string();

    if novel_root.join(".git").exists() {
        return Ok(GitInitResult {
            ok: true,
            repo_exists: true,
            summary: "Git 仓库已存在".into(),
        });
    }
    match Command::new("git").args(["-C", &dir, "init"]).output() {
        Ok(o) if o.status.success() => Ok(GitInitResult {
            ok: true,
            repo_exists: true,
            summary: "已初始化 Git 仓库".into(),
        }),
        Ok(o) => Err(format!(
            "git init 失败：{}",
            truncate_str(&String::from_utf8_lossy(&o.stderr), 200)
        )),
        Err(e) => Err(format!("git init 失败：{e}")),
    }
}

/// 截断字符串到最多 n 个字符（按字符边界，避免切坏 UTF-8）。
fn truncate_str(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// 解析 `### FILE: <path>\n<content>\n### END` 块
fn split_file_blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current_path: Option<String> = None;
    let mut buf = String::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("### FILE:") {
            // 新块开始
            if let (Some(p), content) = (current_path.take(), std::mem::take(&mut buf)) {
                out.push((p, content));
            }
            current_path = Some(path.trim().to_string());
            buf = String::new();
        } else if line.trim() == "### END" {
            if let (Some(p), content) = (current_path.take(), std::mem::take(&mut buf)) {
                out.push((p, content));
            }
        } else {
            if current_path.is_some() {
                buf.push_str(line);
                buf.push('\n');
            }
        }
    }
    // flush
    if let (Some(p), content) = (current_path.take(), std::mem::take(&mut buf)) {
        out.push((p, content));
    }
    out
}
fn infer_chapter_number(text: &str) -> Option<u32> {
    for line in text.lines() {
        let t = line.trim_start_matches('#').trim();
        if t.is_empty() {
            continue;
        }
        // 匹配 "第一章" / "第 3 章" / "第十二章" / "ch1"
        if let Some(rest) = t.strip_prefix("第") {
            // 取到 "章" 之前的部分
            let part: String = rest.chars().take_while(|c| *c != '章').collect();
            let part = part.trim();
            if let Some(n) = parse_cn_num(part) {
                return Some(n);
            }
            // 数字形式 "第 12 章"
            let digits: String = part.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                return Some(n);
            }
        }
        // "ch1" / "ch01" / "ch001"
        let lower = t.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("ch") {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                return Some(n);
            }
        }
        return None; // 第一个非空标题行没有章号 → 不推断
    }
    None
}

/// 解析中文数字（一到九十九；百/千 支持到 9999）。
fn parse_cn_num(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // 纯阿拉伯数字
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let digit = |c: char| -> Option<u32> {
        match c {
            '零' | '〇' => Some(0),
            '一' | '壹' => Some(1),
            '二' | '两' | '贰' => Some(2),
            '三' | '叁' => Some(3),
            '四' | '肆' => Some(4),
            '五' | '伍' => Some(5),
            '六' | '陆' => Some(6),
            '七' | '柒' => Some(7),
            '八' | '捌' => Some(8),
            '九' | '玖' => Some(9),
            _ => return None,
        }
        .into()
    };
    // 简单处理：十/百/千
    let mut total: u32 = 0;
    let mut current: u32 = 0;
    let mut last_digit: Option<u32> = None;
    for c in s.chars() {
        match c {
            '十' => {
                let base = if current == 0 { 1 } else { current };
                total += base * 10;
                current = 0;
                last_digit = None;
            }
            '百' => {
                let base = if current == 0 { 1 } else { current };
                total += base * 100;
                current = 0;
                last_digit = None;
            }
            '千' => {
                let base = if current == 0 { 1 } else { current };
                total += base * 1000;
                current = 0;
                last_digit = None;
            }
            _ => {
                if let Some(d) = digit(c) {
                    current = d;
                    last_digit = Some(d);
                } else {
                    return None;
                }
            }
        }
    }
    if let Some(d) = last_digit {
        total += d;
    }
    if total > 0 {
        Some(total)
    } else {
        None
    }
}

/// 更新 state.yml 的 current_chapter（简单行替换）。
fn update_state_current_chapter(root: &Path, next: u32) -> Result<(), String> {
    let path = root.join("state.yml");
    let text = fs::read_to_string(&path).map_err(|e| format!("read state.yml: {e}"))?;
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut replaced = false;
    for line in lines.iter_mut() {
        let t = line.trim();
        if t.starts_with("current_chapter:") {
            let indent = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>();
            *line = format!("{indent}current_chapter: {next}");
            replaced = true;
            break;
        }
    }
    if !replaced {
        lines.push(format!("current_chapter: {next}"));
    }
    fs::write(&path, lines.join("\n").as_bytes())
        .map_err(|e| format!("write state.yml: {e}"))
}

/// 去掉输出里的 @@CHOICE@@ 标记（如果混在文本里），保留正文。
fn strip_choice_markers(text: &str) -> String {
    let mut out = String::new();
    let mut in_choice = false;
    let mut rest = text;
    loop {
        let start = rest.find("@@CHOICE@@");
        match start {
            Some(idx) => {
                out.push_str(&rest[..idx]);
                let after = &rest[idx + "@@CHOICE@@".len()..];
                match after.find("@@END@@") {
                    Some(end) => {
                        rest = &after[end + "@@END@@".len()..];
                    }
                    None => {
                        in_choice = true;
                        break;
                    }
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    if in_choice {
        // 文本末尾还有未闭合的 CHOICE —— 截断到标记前
        // （这种情况不该发生，防御性处理）
    }
    out.trim().to_string()
}

/// 提取章节 markdown 正文：跳过 agent 的"说明文字"，从第一个 `# ` 标题开始。
/// 若找不到标题，原样返回。
fn extract_markdown_body(text: &str) -> String {
    let stripped = strip_choice_markers(text);
    let mut lines = stripped.lines();
    // 跳过正文前的说明文字（直到第一个以 # 开头的一级/多级标题行）
    for (i, line) in stripped.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("# ") || t.starts_with("## ") || t.starts_with("### ") {
            return stripped
                .lines()
                .skip(i)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
        }
        let _ = lines.next();
    }
    stripped
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
            probe_directory,
            read_choice_points,
            decide_choice_point,
            create_choice_point,
            seed_demo_choice_points,
            ai_write_chapter,
            ai_reconcile_bible,
            ai_review_chapter,
            ai_revise_chapter,
            ai_full_pipeline,
            read_story_spine,
            git_commit,
            git_init,
            read_changes,
            dsh_login,
            dsh_logout,
            dsh_login_status
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
    use std::sync::Mutex;

    /// 串行化所有访问真实用户目录的测试（避免并行时 .ai-novel 互踩）。
    static REAL_DIR_LOCK: Mutex<()> = Mutex::new(());
    fn lock_real_dir() -> std::sync::MutexGuard<'static, ()> {
        REAL_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

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

    // ------------------------------------------------------------------
    // ChoicePoint 命令：seed → list → decide → 持久化
    // ------------------------------------------------------------------

    #[test]
    fn choice_seed_list_decide_roundtrip() {
        // 起一个临时 novel 项目根（含 state.yml + bible/，让 find_novel_root 命中）
        let root = std::env::temp_dir().join("dsh-novel-cp-roundtrip");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bible").join("characters")).unwrap();
        fs::write(root.join("state.yml"), "title: 测试\ncurrent_chapter: 0\n").unwrap();

        // seed 应当建出至少 1 个 cp；这里因为 .ai-novel 不存在，调用应该补 3 个
        let added = seed_demo_choice_points(root.to_string_lossy().to_string())
            .expect("seed ok");
        assert!(added >= 1, "seed added={added}");

        // list 应该看到条目
        let view = read_choice_points(root.to_string_lossy().to_string()).expect("read ok");
        assert!(view.ai_novel_dir_exists, ".ai-novel 没创建");
        assert!(!view.points.is_empty(), "应该至少有 1 个抉择点");

        // 找第一个 pending 抉择点，做出决定（cp-002 是 major，但 seed 里没决定）
        let pending = view
            .points
            .iter()
            .find(|p| p.decided.is_none())
            .expect("至少 1 个 pending");
        let decision = decide_choice_point(DecideChoiceArgs {
            root: root.to_string_lossy().to_string(),
            point_id: pending.id.clone(),
            option_id: "ai".to_string(),
            by: "human".to_string(),
            note: Some("决定：让 AI 选".to_string()),
        })
        .expect("decide ok");
        assert_eq!(decision.by, "human");
        assert_eq!(decision.option_id, "ai");

        // 再读一次，确认 persisted
        let view2 = read_choice_points(root.to_string_lossy().to_string()).expect("reread ok");
        assert_eq!(view2.decided_count, view.decided_count + 1);
        assert_eq!(view2.pending_count, view.pending_count - 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn choice_decide_rejects_invalid_option() {
        let root = std::env::temp_dir().join("dsh-novel-cp-reject");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bible")).unwrap();
        fs::write(root.join("state.yml"), "title: t\n").unwrap();

        let _ = seed_demo_choice_points(root.to_string_lossy().to_string()).unwrap();

        // 取第一个 cp（cp-001 已定，cp-002 未定），用一个不存在的 option 决定
        let view = read_choice_points(root.to_string_lossy().to_string()).unwrap();
        let pending = view.points.iter().find(|p| p.decided.is_none()).unwrap();
        let result = decide_choice_point(DecideChoiceArgs {
            root: root.to_string_lossy().to_string(),
            point_id: pending.id.clone(),
            option_id: "z-non-existent".to_string(),
            by: "human".to_string(),
            note: None,
        });
        assert!(result.is_err(), "invalid option 应该报错，got {:?}", result);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn choice_create_requires_ai_option() {
        let root = std::env::temp_dir().join("dsh-novel-cp-create");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bible")).unwrap();
        fs::write(root.join("state.yml"), "title: t\n").unwrap();

        // 不含 id=ai 的应该报错
        let bad = create_choice_point(CreateChoicePointArgs {
            root: root.to_string_lossy().to_string(),
            weight: "major".to_string(),
            after_chapter: "ch2".to_string(),
            prompt: "测试无 ai 选项".to_string(),
            options: vec![
                ChoiceOption { id: "a".to_string(), label: "a".to_string(), preview_hint: "...".to_string() },
                ChoiceOption { id: "b".to_string(), label: "b".to_string(), preview_hint: "...".to_string() },
            ],
            decided: None,
        });
        assert!(bad.is_err(), "应该要求 ai 选项");

        let _ = fs::remove_dir_all(&root);
    }

    /// 真实用户目录 smoke：seed → list → decide → 持久化 → 清理。
    /// 不依赖 GUI；直接调私有命令函数。
    #[test]
    fn smoke_choice_points_on_real_user_dir() {
        let _guard = lock_real_dir();
        let p = "/Users/zhouluyong/Documents/我的小说";
        let path = std::path::Path::new(p);
        if !path.join("state.yml").exists() || !path.join("bible").is_dir() {
            eprintln!("smoke skipped — 项目根未就绪（state.yml/bible/ 缺失）");
            return;
        }
        let added = seed_demo_choice_points(p.to_string()).expect("seed ok");
        eprintln!("seed: added = {added}");

        let view = read_choice_points(p.to_string()).expect("read ok");
        eprintln!(
            "before decide: total={} decided={} pending={}",
            view.points.len(),
            view.decided_count,
            view.pending_count
        );
        assert!(view.ai_novel_dir_exists);
        assert!(!view.points.is_empty());

        if let Some(pending) = view.points.iter().find(|p| p.decided.is_none()) {
            let dec = decide_choice_point(DecideChoiceArgs {
                root: p.to_string(),
                point_id: pending.id.clone(),
                option_id: "ai".to_string(),
                by: "human".to_string(),
                note: Some("smoke".to_string()),
            })
            .expect("decide ok");
            eprintln!("decide: cp={} option={} by={}", pending.id, dec.option_id, dec.by);
        }

        let view2 = read_choice_points(p.to_string()).expect("reread ok");
        eprintln!(
            "after decide:  total={} decided={} pending={}",
            view2.points.len(),
            view2.decided_count,
            view2.pending_count
        );
        assert!(view2.decided_count >= 1);
        eprintln!("✓ {}/.ai-novel 写入了 demo 抉择点", p);

        // 清理 smoke 数据（让用户从 UI 真正走一遍时是干净的）
        let _ = fs::remove_dir_all(path.join(".ai-novel"));
        eprintln!("(清理 .ai-novel smoke 用例数据)");
    }

    /// git init → commit → 再 commit（无改动）的完整回路。不依赖 GUI，纯本地 git。
    #[test]
    fn git_init_commit_roundtrip() {
        use std::process::Command;
        let _guard = lock_real_dir();
        let p = std::env::temp_dir().join("dsh-novel-git-test");
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("bible")).unwrap();
        fs::create_dir_all(p.join("chapters")).unwrap();
        fs::write(p.join("state.yml"), "title: t\ncurrent_chapter: 1\n").unwrap();
        fs::write(p.join("chapters").join("ch01.md"), "# 第一章\n正文\n").unwrap();
        let root = p.to_string_lossy().to_string();

        // 未 init → repo_exists=false
        let r = git_commit(root.clone(), Some("m".into())).expect("commit call");
        assert!(!r.repo_exists, "未初始化应报 repo_exists=false，got {}", r.summary);

        let gi = git_init(root.clone()).expect("init ok");
        assert!(gi.ok && gi.repo_exists, "init 应成功");

        // 保证本地身份，避免 commit 因缺 user.name/email 失败
        let _ = Command::new("git").args(["-C", root.as_str(), "config", "user.name", "test"]).output();
        let _ = Command::new("git").args(["-C", root.as_str(), "config", "user.email", "test@example.com"]).output();

        let r2 = git_commit(root.clone(), Some("里程碑".into())).expect("commit call");
        assert!(r2.committed, "首次提交应成功，got {}", r2.summary);
        assert!(r2.hash.is_some(), "应返回短 hash");

        let r3 = git_commit(root.clone(), Some("again".into())).expect("commit call");
        assert!(!r3.committed, "无改动不应再提交");
        assert!(r3.summary.contains("没有需要提交的改动"), "got {}", r3.summary);

        let _ = fs::remove_dir_all(&p);
    }

    /// 写前脏检查：指纹一致 → 覆盖成功；指纹不一致 → conflict；None → 强制覆盖。
    #[test]
    fn write_chapter_dirty_check() {
        let _guard = lock_real_dir();
        let p = std::env::temp_dir().join("dsh-novel-write-check");
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("bible")).unwrap();
        fs::create_dir_all(p.join("chapters")).unwrap();
        fs::write(p.join("state.yml"), "title: t\ncurrent_chapter: 1\n").unwrap();
        fs::write(p.join("chapters").join("ch01.md"), "# 第一章\n旧\n").unwrap();
        let root = p.to_string_lossy().to_string();
        let chapter_path = p.join("chapters").join("ch01.md");

        let fp = file_fingerprint(&chapter_path).expect("指纹应存在");

        // 指纹一致 → 覆盖成功
        let r1 = write_chapter(WriteChapterArgs {
            root: root.clone(),
            file: "ch01.md".into(),
            content: "# 第一章\n新正文更长\n".into(),
            base_fingerprint: Some(fp),
        })
        .expect("write ok");
        assert!(r1.ok && !r1.conflict, "指纹一致应成功，got {:?}", (r1.ok, r1.conflict));

        // 指纹不一致（旧指纹）→ conflict
        let r2 = write_chapter(WriteChapterArgs {
            root: root.clone(),
            file: "ch01.md".into(),
            content: "覆盖".into(),
            base_fingerprint: Some(fp),
        })
        .expect("write ok");
        assert!(!r2.ok && r2.conflict, "指纹已过期应冲突，got {:?}", (r2.ok, r2.conflict));

        // None → 强制覆盖
        let r3 = write_chapter(WriteChapterArgs {
            root: root.clone(),
            file: "ch01.md".into(),
            content: "强制覆盖".into(),
            base_fingerprint: None,
        })
        .expect("write ok");
        assert!(r3.ok && !r3.conflict, "强制覆盖应成功");

        let _ = fs::remove_dir_all(&p);
    }

    /// 变更检测：首次播种 → 无变化；改动后 → changed=true；stamps 覆盖 state.yml/chapters。
    #[test]
    fn read_changes_detects_external_edit() {
        let _guard = lock_real_dir();
        let p = std::env::temp_dir().join("dsh-novel-changes-test");
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("bible")).unwrap();
        fs::create_dir_all(p.join("chapters")).unwrap();
        fs::write(p.join("state.yml"), "title: t\ncurrent_chapter: 1\n").unwrap();
        fs::write(p.join("chapters").join("ch01.md"), "# 第一章\n正文\n").unwrap();
        let root = p.to_string_lossy().to_string();

        let seed = read_changes(ReadChangesArgs { root: root.clone(), last_seen: None }).expect("seed");
        assert!(!seed.changed, "首次播种不应算变化");
        assert!(seed.stamps.iter().any(|s| s.path == "state.yml"));
        assert!(seed.stamps.iter().any(|s| s.path == "chapters/ch01.md"));

        // 无改动 → changed=false
        let same = read_changes(ReadChangesArgs { root: root.clone(), last_seen: Some(seed.stamps.clone()) }).expect("same");
        assert!(!same.changed, "无改动不应报变化");

        // 外部改动章节 → changed=true
        fs::write(p.join("chapters").join("ch01.md"), "# 第一章\n改过了\n").unwrap();
        let diff = read_changes(ReadChangesArgs { root: root.clone(), last_seen: Some(seed.stamps) }).expect("diff");
        assert!(diff.changed, "改动后应报变化");

        let _ = fs::remove_dir_all(&p);
    }

    /// 端到端：真实驱动 DSH agent 写一章（需要 dsh web 在 3080 跑着）。
    /// 默认 #[ignore]，手动跑：cargo test e2e_ai_write_chapter -- --ignored --nocapture
    #[test]
    #[ignore]
    fn e2e_ai_write_chapter() {
        let p = "/Users/zhouluyong/Documents/我的小说";
        let path = std::path::Path::new(p);
        if !path.join("state.yml").exists() || !path.join("bible").is_dir() {
            eprintln!("skipped — 项目根未就绪");
            return;
        }
        let result = ai_write_chapter(AiWriteChapterArgs {
            root: p.to_string(),
            instruction: Some("写第一章（ch001.md），大约 600-1000 字即可（测试用短章）。主角登场，结尾留一个钩子。".to_string()),
            session_id: None,
            timeout_secs: Some(240),
            port: Some(3080),
        })
        .expect("ai_write_chapter call");

        eprintln!("ok={} saved_to={:?} session={:?}", result.ok, result.saved_to, result.session_id);
        eprintln!("choice_request={:?}", result.choice_request.as_ref().map(|c| c.get("prompt")));
        eprintln!("text_len={}", result.text.chars().count());
        eprintln!("text_head: {}", result.text.chars().take(200).collect::<String>());

        // 若保存了章节，回读验证
        if let Some(file) = result.saved_to {
            let fp = path.join("chapters").join(&file);
            let content = fs::read_to_string(&fp).unwrap_or_default();
            assert!(!content.trim().is_empty(), "保存的章节不应为空");
            eprintln!("✓ 已保存章节 {file}（{} 字）", content.chars().count());
        }
    }

    #[test]
    fn infer_chapter_number_variants() {
        assert_eq!(infer_chapter_number("# 第一章 停在三点四十七"), Some(1));
        assert_eq!(infer_chapter_number("# 第十二章 重逢"), Some(12));
        assert_eq!(infer_chapter_number("# 第 3 章 迷雾"), Some(3));
        assert_eq!(infer_chapter_number("# ch5 后记"), Some(5));
        assert_eq!(infer_chapter_number("## 序章"), None); // 序章不是数字
        assert_eq!(infer_chapter_number("随便一段文字没有标题"), None);
    }

    #[test]
    fn parse_cn_num_variants() {
        assert_eq!(parse_cn_num("一"), Some(1));
        assert_eq!(parse_cn_num("十二"), Some(12));
        assert_eq!(parse_cn_num("二十"), Some(20));
        assert_eq!(parse_cn_num("二十一"), Some(21));
        assert_eq!(parse_cn_num("百"), Some(100));
        assert_eq!(parse_cn_num("三百"), Some(300));
        assert_eq!(parse_cn_num("三十二"), Some(32));
    }

    #[test]
    fn extract_markdown_body_drops_preamble() {
        let text = "第一章已写好并保存。\n\n按任务要求没有修改文件。\n\n---\n\n# 第一章 怀表\n\n正文开始了。";
        let body = extract_markdown_body(text);
        assert!(body.starts_with("# 第一章 怀表"), "body={body}");
        assert!(!body.contains("已写好"), "不应含说明文字");
        assert!(body.contains("正文开始了"));
    }

    #[test]
    fn split_file_blocks_parses_marker_blocks() {
        let text = "先说一下。\n\n### FILE: bible/timeline.md\n| ch1 | 世界历 47 年秋 | 17 | 王城 | 李四捡到怀表 | 李四 |\n### END\n\n### FILE: bible/foreshadowing.md\n(unchanged)\n### END";
        let blocks = split_file_blocks(text);
        assert_eq!(blocks.len(), 2, "blocks={blocks:?}");
        assert_eq!(blocks[0].0, "bible/timeline.md");
        assert!(blocks[0].1.contains("李四捡到怀表"));
        assert_eq!(blocks[1].0, "bible/foreshadowing.md");
        assert_eq!(blocks[1].1.trim(), "(unchanged)");
    }

    /// 端到端：read_story_spine 真实项目。无需 dsh web。
    #[test]
    fn e2e_read_story_spine_real_dir() {
        let _guard = lock_real_dir();
        let p = "/Users/zhouluyong/Documents/我的小说";
        let path = std::path::Path::new(p);
        if !path.join("state.yml").exists() || !path.join("bible").is_dir() {
            eprintln!("skipped — 项目根未就绪");
            return;
        }
        // 确保有抉择点数据（seed 若没有）
        let _ = seed_demo_choice_points(p.to_string()).expect("seed");
        let spine = read_story_spine(p.to_string()).expect("read spine");
        eprintln!("nodes: {}", spine.nodes.len());
        for n in &spine.nodes {
            eprintln!("  [{}] {} {}", n.kind, n.id, n.title.chars().take(30).collect::<String>());
        }
        assert!(!spine.nodes.is_empty());
        // 清理 seed（保持目录干净）
        let _ = fs::remove_dir_all(path.join(".ai-novel"));
        eprintln!("(清理 .ai-novel)");
    }

    /// 审核报告 JSON 解析（用真实 agent 输出格式的样本）。
    #[test]
    fn review_report_parse_sample() {
        let sample = r#"{
  "summary": "本章整体流畅，但有 2 处需要注意",
  "verdict": "revise",
  "categories": {
    "A": {"label": "严重冲突", "issues": [{"severity": "major", "location": "第 3 段", "issue": "李四已经死了但本章又登场", "suggestion": "改回他活着的时间线"}]},
    "B": {"label": "性格漂移", "issues": []},
    "C": {"label": "信息差漏洞", "issues": [{"severity": "info", "location": "对话段", "issue": "李四知道了北境魔宗的内部密语", "suggestion": "让对话由魔宗内应说出"}]},
    "D": {"label": "伏笔推进", "issues": []},
    "P": {"label": "节奏问题", "issues": [{"severity": "minor", "location": "章末", "issue": "钩子较弱", "suggestion": "末尾加一句危机预告"}]}
  }
}"#;
        let json_str = extract_json_object(sample).expect("extract json");
        let v: Value = serde_json::from_str(&json_str).expect("parse");
        assert_eq!(v["verdict"], "revise");
        let cats = v["categories"].as_object().expect("categories object");
        assert!(cats.contains_key("A") && cats.contains_key("B"));
        let a_issues = cats["A"]["issues"].as_array().expect("A issues");
        assert_eq!(a_issues.len(), 1);
        assert_eq!(a_issues[0]["severity"], "major");
        assert_eq!(cats["C"]["issues"][0]["issue"].as_str().unwrap().contains("密语"), true);
    }

    /// 端到端：一键流水线（写→审→改→收尾）。需要 dsh web。默认 #[ignore]。
    /// 谨慎运行：会真实写入新章节 + 修改 bible。用短指令控制篇幅。
    #[test]
    #[ignore]
    fn e2e_ai_full_pipeline() {
        let p = "/Users/zhouluyong/Documents/我的小说";
        let path = std::path::Path::new(p);
        if !path.join("state.yml").exists() || !path.join("bible").is_dir() {
            eprintln!("skipped — 项目根未就绪");
            return;
        }
        // 用"写序章测试篇"避免推进主线
        let res = ai_full_pipeline(AiFullPipelineArgs {
            root: p.to_string(),
            instruction: Some("写一章序章测试（约300-500字即可，测试用短章）。不要推进主线剧情，只写环境与人物速写。".to_string()),
            auto_revise: Some(true),
            auto_reconcile: Some(true),
            session_id: None,
            timeout_secs: Some(600),
            port: Some(3080),
        })
        .expect("pipeline call");
        eprintln!("stage={} ok={}", res.stage, res.ok);
        eprintln!("chapter={:?} verdict={:?}", res.chapter_file, res.verdict);
        eprintln!("summary={:?}", res.review_summary);
        eprintln!("reconcile={:?}", res.reconcile_note);
        eprintln!("final_text_len={}", res.final_text.as_ref().map(|t| t.chars().count()).unwrap_or(0));
        if let Some(err) = &res.error {
            eprintln!("error={err}");
        }
    }

    /// 端到端：AI 审核最近一章。需要 dsh web。默认 #[ignore]。
    #[test]
    #[ignore]
    fn e2e_ai_review_chapter() {
        let p = "/Users/zhouluyong/Documents/我的小说";
        let path = std::path::Path::new(p);
        if !path.join("state.yml").exists() || !path.join("bible").is_dir() {
            eprintln!("skipped — 项目根未就绪");
            return;
        }
        let report = ai_review_chapter(AiReviewChapterArgs {
            root: p.to_string(),
            chapter_file: None,
            session_id: None,
            timeout_secs: Some(240),
            port: Some(3080),
        })
        .expect("review call");
        eprintln!("verdict={} summary={}", report.verdict, report.summary);
        for (key, cat) in &report.categories {
            eprintln!("  [{key}] {} issues={}", cat.label, cat.issues.len());
        }
    }

    /// 端到端：AI 收尾三件事（同步圣经）。需要 dsh web。默认 #[ignore]。
    #[test]
    #[ignore]
    fn e2e_ai_reconcile_bible() {
        let p = "/Users/zhouluyong/Documents/我的小说";
        let path = std::path::Path::new(p);
        if !path.join("state.yml").exists() || !path.join("bible").is_dir() {
            eprintln!("skipped — 项目根未就绪");
            return;
        }
        // 备份 timeline/foreshadowing（防止真改坏）
        let tl_bak = fs::read_to_string(path.join("bible").join("timeline.md")).unwrap_or_default();
        let fs_bak = fs::read_to_string(path.join("bible").join("foreshadowing.md")).unwrap_or_default();

        let res = ai_reconcile_bible(AiReconcileBibleArgs {
            root: p.to_string(),
            chapter_file: Some("ch001.md".to_string()),
            decision: Some("cp-001: 人类决定选 b（流放萧承）".to_string()),
            session_id: None,
            timeout_secs: Some(240),
            port: Some(3080),
        })
        .expect("reconcile call");

        eprintln!("ok={} error={:?} text_len={}", res.ok, res.error, res.text.chars().count());
        eprintln!("text_head: {}", res.text.chars().take(300).collect::<String>());

        // 恢复备份
        fs::write(path.join("bible").join("timeline.md"), tl_bak.as_bytes()).unwrap();
        fs::write(path.join("bible").join("foreshadowing.md"), fs_bak.as_bytes()).unwrap();
        eprintln!("(已恢复 timeline/foreshadowing 备份)");
    }
}
