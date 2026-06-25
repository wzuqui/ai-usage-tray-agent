// Janela de novidades da atualizacao (OTA). Mostra "versao atual -> nova" e o
// "delta" do changelog (todas as versoes entre a instalada e a mais nova, lidas
// do CHANGELOG.md via `get_changelog`), e oferece instalar agora com barra de
// progresso. Versoes/instalacao vem de `get_pending_update`/`install_update`
// (que reinicia o app ao concluir, entao o sucesso nunca "volta" para ca).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { compareVersions, escapeHtml, parseChangelog, renderMarkdown } from "./changelog";

interface PendingUpdate {
  appName: string;
  currentVersion: string;
  newVersion: string;
  notes: string;
}

interface ProgressPayload {
  downloaded: number;
  total: number | null;
}

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const win = getCurrentWindow();

// Monta o "delta": todas as versoes do changelog com instalada < v <= nova. A
// secao [Nao lancado] e' a versao mais nova (ainda nao promovida), entao a
// mapeamos para `newVersion`. Retorna null se nao houver nada a mostrar.
function renderDelta(md: string, currentVersion: string, newVersion: string): string | null {
  const seen = new Set<string>();
  const delta = parseChangelog(md)
    .map((entry) => ({ entry, version: entry.isUnreleased ? newVersion : entry.version }))
    .filter(({ entry, version }) => {
      if (!version || !entry.body.trim()) return false;
      if (seen.has(version)) return false;
      seen.add(version);
      return compareVersions(version, currentVersion) > 0 && compareVersions(version, newVersion) <= 0;
    })
    .sort((a, b) => compareVersions(b.version!, a.version!));

  if (delta.length === 0) return null;

  return delta
    .map(({ entry, version }) => {
      const date = entry.date ? ` <span class="cl-date">· ${escapeHtml(entry.date)}</span>` : "";
      return `<section class="cl-entry"><h2 class="cl-ver">${escapeHtml(version!)}${date}</h2>${renderMarkdown(entry.body)}</section>`;
    })
    .join("\n");
}

function fallbackNotes(notes: string): string {
  const trimmed = (notes ?? "").trim();
  return trimmed
    ? renderMarkdown(trimmed)
    : `<p class="foot">Não foi possível carregar as novidades. Você ainda pode atualizar.</p>`;
}

function setBusy(busy: boolean): void {
  ($("upd-install") as HTMLButtonElement).disabled = busy;
  ($("upd-later") as HTMLButtonElement).disabled = busy;
  $("upd-progress").classList.toggle("hide", !busy);
}

async function install(): Promise<void> {
  setBusy(true);
  $("upd-status").textContent = "";
  $("upd-progress-label").textContent = "Iniciando download…";
  try {
    // Resolve so' em caso de falha: no sucesso o backend reinicia o app.
    await invoke("install_update");
  } catch (error) {
    setBusy(false);
    $("upd-status").textContent = String(error);
  }
}

async function init(): Promise<void> {
  // Progresso do download emitido pelo backend.
  await listen<ProgressPayload>("update-progress", (event) => {
    const { downloaded, total } = event.payload;
    const fill = $("upd-bar-fill");
    if (total && total > 0) {
      const pct = Math.min(100, Math.round((downloaded / total) * 100));
      fill.style.width = `${pct}%`;
      $("upd-progress-label").textContent =
        `Baixando… ${pct}% (${formatMB(downloaded)} de ${formatMB(total)})`;
    } else {
      // Sem Content-Length: barra indeterminada (mostra bytes baixados).
      fill.style.width = "100%";
      $("upd-progress-label").textContent = `Baixando… ${formatMB(downloaded)}`;
    }
  });

  let data: PendingUpdate | null = null;
  try {
    data = await invoke<PendingUpdate | null>("get_pending_update");
  } catch {
    data = null;
  }

  if (!data) {
    $("upd-versions").textContent = "Não há dados da atualização.";
    $("upd-notes").innerHTML = "";
    ($("upd-install") as HTMLButtonElement).disabled = true;
    return;
  }

  $("upd-versions").innerHTML =
    `Versão atual <strong>${escapeHtml(data.currentVersion)}</strong> → ` +
    `nova versão <strong>${escapeHtml(data.newVersion)}</strong>`;

  $("upd-install").addEventListener("click", () => void install());
  $("upd-later").addEventListener("click", () => void win.close());

  // Novidades: busca o CHANGELOG.md e monta o delta de versões. Falha de rede
  // cai no fallback (notes do manifesto, se houver, ou um aviso) sem travar.
  $("upd-notes").innerHTML = `<p class="foot">Carregando novidades…</p>`;
  try {
    const md = await invoke<string>("get_changelog");
    $("upd-notes").innerHTML = renderDelta(md, data.currentVersion, data.newVersion)
      ?? fallbackNotes(data.notes);
  } catch {
    $("upd-notes").innerHTML = fallbackNotes(data.notes);
  }
}

function formatMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

void init();
