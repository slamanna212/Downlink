// Release notes arrive as the GitHub release body (Markdown). We only need headings, bullets and paragraphs,
// so this turns them into plain blocks instead of pulling in a Markdown renderer. Inline formatting
// (**bold**, `code`, [text](url)) is reduced to its text; links are never rendered as links.

export type NotesBlock =
  | { kind: 'heading'; text: string }
  | { kind: 'list'; items: string[] }
  | { kind: 'para'; text: string };

function inline(s: string): string {
  return s
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/(\*\*|__)(.+?)\1/g, '$2')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

export function parseReleaseNotes(md: string): NotesBlock[] {
  const blocks: NotesBlock[] = [];
  let para: string[] = [];
  const flush = () => {
    const text = inline(para.join(' '));
    if (text) blocks.push({ kind: 'para', text });
    para = [];
  };
  for (const raw of md.replace(/\r\n?/g, '\n').split('\n')) {
    const line = raw.trim();
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    const bullet = /^[-*+]\s+(.*)$/.exec(line);
    if (!line || /^(-{3,}|\*{3,}|_{3,})$/.test(line)) {
      flush();
    } else if (heading) {
      flush();
      const text = inline(heading[1] ?? '');
      if (text) blocks.push({ kind: 'heading', text });
    } else if (bullet) {
      flush();
      const text = inline(bullet[1] ?? '');
      if (!text) continue;
      const last = blocks[blocks.length - 1];
      if (last?.kind === 'list') last.items.push(text);
      else blocks.push({ kind: 'list', items: [text] });
    } else {
      para.push(line);
    }
  }
  flush();
  return blocks;
}
