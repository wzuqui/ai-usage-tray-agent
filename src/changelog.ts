// Utilidades de changelog compartilhadas pela janela OTA (update.ts) e pela tela
// "Novidades" (novidades.ts): renderizador de Markdown mínimo/seguro, parser das
// seções de versão e comparação de versões. A fonte é o CHANGELOG.md (buscado no
// backend via `get_changelog`).

export interface ChangelogEntry {
  /** "0.2.26" para seções de versão; null quando é a [Não lançado]. */
  version: string | null;
  /** "2026-06-24" quando presente no cabeçalho; senão null. */
  date: string | null;
  /** true para a seção [Não lançado] (a versão mais nova, ainda não promovida). */
  isUnreleased: boolean;
  /** Corpo em Markdown (categorias ### + itens). */
  body: string;
}

// ===== Renderizador de Markdown mínimo (e seguro) =====
// Suporta só o necessário para o changelog: títulos (#..####), listas (-/*),
// **negrito**, `código` e parágrafos. O conteúdo é escapado ANTES de qualquer
// substituição, então não há injeção de HTML mesmo que o changelog mude.
export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function inline(s: string): string {
  return s
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    // Links: mostra apenas o texto (sem navegar a webview p/ fora do app).
    .replace(/\[([^\]]+)\]\((?:https?:\/\/[^)\s]+)\)/g, "$1");
}

export function renderMarkdown(md: string): string {
  const lines = escapeHtml(md.replace(/\r\n/g, "\n")).split("\n");
  const out: string[] = [];
  let inList = false;
  let para: string[] = [];
  const closeList = () => {
    if (inList) {
      out.push("</ul>");
      inList = false;
    }
  };
  const flushPara = () => {
    if (para.length) {
      out.push(`<p>${inline(para.join(" "))}</p>`);
      para = [];
    }
  };
  for (const raw of lines) {
    const line = raw.trim();
    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    const item = /^[-*]\s+(.*)$/.exec(line);
    if (heading) {
      flushPara();
      closeList();
      const level = Math.min(heading[1].length + 1, 5);
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
    } else if (item) {
      flushPara();
      if (!inList) {
        out.push("<ul>");
        inList = true;
      }
      out.push(`<li>${inline(item[1])}</li>`);
    } else if (line === "") {
      flushPara();
      closeList();
    } else {
      para.push(line);
    }
  }
  flushPara();
  closeList();
  return out.join("\n");
}

// ===== Parser das seções de versão =====
// Cada cabeçalho `## ` (h2) inicia uma entrada. `### ` (categorias) e o restante
// do conteúdo entram no corpo. Cabeçalhos que não são versão nem [Não lançado]
// (ex.: "Histórico") são ignorados, junto com seu corpo. O intro/blockquote (`>`,
// `#`) antes da 1ª seção também é descartado.
export function parseChangelog(md: string): ChangelogEntry[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const entries: ChangelogEntry[] = [];
  let cur: ChangelogEntry | null = null;
  let body: string[] = [];
  const flush = () => {
    if (cur) {
      cur.body = body.join("\n").trim();
      entries.push(cur);
    }
    body = [];
  };
  for (const line of lines) {
    // `^##\s` casa só h2 ("## "), não h3 ("### ") — após "##" vem "#", não espaço.
    const h2 = /^##\s+(.*)$/.exec(line);
    if (h2) {
      const title = h2[1].trim();
      const ver = /^\[?(\d+\.\d+\.\d+)\]?\s*(?:[-–]\s*(.*))?$/.exec(title);
      const unreleased = /n[aã]o\s+lan[çc]ado|unreleased/i.test(title);
      if (ver) {
        flush();
        cur = { version: ver[1], date: (ver[2] ?? "").trim() || null, isUnreleased: false, body: "" };
      } else if (unreleased) {
        flush();
        cur = { version: null, date: null, isUnreleased: true, body: "" };
      } else {
        // Seção não-versão (ex.: "Histórico"): encerra a atual e para de coletar.
        flush();
        cur = null;
      }
    } else if (cur) {
      body.push(line);
    }
  }
  flush();
  return entries;
}

/** Compara versões "x.y.z" numericamente. >0 se a>b, <0 se a<b, 0 se iguais. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map((n) => parseInt(n, 10) || 0);
  const pb = b.split(".").map((n) => parseInt(n, 10) || 0);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}
