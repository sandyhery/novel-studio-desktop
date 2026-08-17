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
// Tauri 命令
// ---------------------------------------------------------------------------

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
            write_chapter
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
