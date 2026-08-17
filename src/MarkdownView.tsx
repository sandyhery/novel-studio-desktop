import type { ReactNode } from "react";

/**
 * 极简 Markdown 渲染器（无外部依赖），面向小说正文：
 * 标题、段落、无序/有序列表、引用块、分隔线，以及行内 **粗体** *斜体* ~~删除~~ `代码`。
 * 不做链接/表格/图片，符合本项目的「轻量」约束。
 */

/** 渲染行内 Markdown（粗体/斜体/删除线/行内代码）。 */
function renderInline(text: string): ReactNode {
  const regex = /(\*\*[^*]+\*\*|\*[^*]+\*|~~[^~]+~~|`[^`]+`)/g;
  const parts: ReactNode[] = [];
  let last = 0;
  let key = 0;
  let m: RegExpExecArray | null;
  while ((m = regex.exec(text)) !== null) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    const tok = m[0];
    if (tok.startsWith("**") && tok.endsWith("**") && tok.length > 4) {
      parts.push(<strong key={key++}>{tok.slice(2, -2)}</strong>);
    } else if (tok.startsWith("~~") && tok.endsWith("~~") && tok.length > 4) {
      parts.push(<del key={key++}>{tok.slice(2, -2)}</del>);
    } else if (tok.startsWith("`") && tok.endsWith("`") && tok.length > 2) {
      parts.push(<code key={key++}>{tok.slice(1, -1)}</code>);
    } else {
      parts.push(<em key={key++}>{tok.slice(1, -1)}</em>);
    }
    last = m.index + tok.length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

function heading(level: number, key: number, content: ReactNode): ReactNode {
  switch (level) {
    case 1: return <h1 key={key}>{content}</h1>;
    case 2: return <h2 key={key}>{content}</h2>;
    case 3: return <h3 key={key}>{content}</h3>;
    case 4: return <h4 key={key}>{content}</h4>;
    case 5: return <h5 key={key}>{content}</h5>;
    default: return <h6 key={key}>{content}</h6>;
  }
}

function isBlockStart(line: string): boolean {
  return (
    /^\s*(#{1,6}\s|>\s|[-*]\s|\d+[.)]\s)/.test(line) ||
    /^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)
  );
}

export default function MarkdownView({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let i = 0;
  let key = 0;

  let listItems: string[] = [];
  let listOrdered: boolean | null = null;

  const flushList = () => {
    if (listItems.length === 0) return;
    const items = listItems;
    const ordered = !!listOrdered;
    listItems = [];
    listOrdered = null;
    blocks.push(
      ordered ? (
        <ol key={key++}>{items.map((it, idx) => <li key={idx}>{renderInline(it)}</li>)}</ol>
      ) : (
        <ul key={key++}>{items.map((it, idx) => <li key={idx}>{renderInline(it)}</li>)}</ul>
      ),
    );
  };

  while (i < lines.length) {
    const line = lines[i];

    // 列表项
    const ulMatch = line.match(/^\s*[-*]\s+(.*)$/);
    const olMatch = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (ulMatch || olMatch) {
      const ordered = !!olMatch;
      if (listOrdered !== null && listOrdered !== ordered) flushList();
      listOrdered = ordered;
      listItems.push((ulMatch || olMatch)![1]);
      i++;
      continue;
    }
    if (listOrdered !== null) flushList();

    // 空行
    if (line.trim() === "") {
      i++;
      continue;
    }

    // 引用块
    if (line.trim().startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        quote.push(lines[i].trim().replace(/^>\s?/, ""));
        i++;
      }
      blocks.push(<blockquote key={key++}>{renderInline(quote.join(" "))}</blockquote>);
      continue;
    }

    // 标题
    const h = line.match(/^(#{1,6})\s+(.+)$/);
    if (h) {
      blocks.push(heading(h[1].length, key++, renderInline(h[2])));
      i++;
      continue;
    }

    // 分隔线
    if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      blocks.push(<hr key={key++} />);
      i++;
      continue;
    }

    // 段落：连续非空、非块起始行合并
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !isBlockStart(lines[i])) {
      para.push(lines[i].trim());
      i++;
    }
    if (para.length) blocks.push(<p key={key++}>{renderInline(para.join(" "))}</p>);
  }
  flushList();

  return <div className="md-view-inner">{blocks}</div>;
}
