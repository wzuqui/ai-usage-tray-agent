// "Novidades": histórico completo de versões, renderizado dentro da aba "Sobre"
// das Configurações (#nov-content). Lê o CHANGELOG.md (via get_changelog) e mostra
// todas as seções de versão; a [Não lançado] aparece como "Mais recente" (a versão
// mais nova ainda não promovida no changelog).
import { invoke } from "@tauri-apps/api/core";
import { escapeHtml, parseChangelog, renderMarkdown } from "./changelog";

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

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

export async function loadNovidades(): Promise<void> {
  const content = $("nov-content");
  content.innerHTML = `<div class="loading">Carregando novidades…</div>`;
  try {
    const md = await invoke<string>("get_changelog");
    content.innerHTML = renderHistory(md);
  } catch {
    content.innerHTML =
      `<p class="foot">Não foi possível carregar as novidades (sem conexão?). Tente recarregar.</p>`;
  }
}
