// Tela "Novidades": histórico completo de versões do app. Lê o CHANGELOG.md (via
// `get_changelog`) e renderiza todas as seções de versão. A [Não lançado] aparece
// como "Mais recente" (a versão mais nova ainda não promovida no changelog).
import { invoke } from "@tauri-apps/api/core";
import { escapeHtml, parseChangelog, renderMarkdown } from "./changelog";

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

let wired = false;

function renderHistory(md: string): string {
  const entries = parseChangelog(md).filter((entry) => entry.body.trim());
  if (entries.length === 0) {
    return `<p class="foot">Nenhuma novidade registrada.</p>`;
  }
  return entries
    .map((entry) => {
      const title = entry.isUnreleased ? "Mais recente" : escapeHtml(entry.version!);
      const date = entry.date ? ` <span class="cl-date">· ${escapeHtml(entry.date)}</span>` : "";
      return `<section class="cl-entry"><h2 class="cl-ver">${title}${date}</h2>${renderMarkdown(entry.body)}</section>`;
    })
    .join("\n");
}

async function load(): Promise<void> {
  const content = $("nov-content");
  $("nov-msg").textContent = "";
  content.innerHTML = `<div class="loading">Carregando novidades…</div>`;
  try {
    const md = await invoke<string>("get_changelog");
    content.innerHTML = renderHistory(md);
  } catch {
    content.innerHTML =
      `<p class="foot">Não foi possível carregar as novidades (sem conexão?). Tente recarregar.</p>`;
  }
}

export function initNovidades(): void {
  // Carrega uma vez por sessão; o changelog raramente muda enquanto o app roda.
  // O botão "Recarregar" força uma nova busca.
  if (wired) return;
  wired = true;
  $("nov-reload").addEventListener("click", () => void load());
  void load();
}
