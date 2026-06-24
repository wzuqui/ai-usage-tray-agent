// Janela de novidades da atualizacao (OTA). Mostra "versao atual -> nova", o
// changelog da versao (campo `notes` do manifesto -> `update.body`, renderizado
// como markdown) e oferece instalar agora, com barra de progresso. Os dados sao
// lidos via `get_pending_update`; o download/instalacao roda em `install_update`
// (que reinicia o app ao concluir, entao o sucesso nunca "volta" para ca).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

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

// ===== Renderizador de markdown minimo (e seguro) =====
// Suporta apenas o necessario para o changelog: titulos (##/###), listas (-/*),
// **negrito**, `codigo` e paragrafos. O conteudo e' escapado ANTES de qualquer
// substituicao, entao nao ha injecao de HTML mesmo que as notas mudem.
function escapeHtml(s: string): string {
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

function renderMarkdown(md: string): string {
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
      // ## no markdown -> h3 na janela (h1 ja' e' o titulo da janela).
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

  const notes = (data.notes ?? "").trim();
  $("upd-notes").innerHTML = notes
    ? renderMarkdown(notes)
    : `<p class="foot">Sem notas para esta versão.</p>`;

  $("upd-install").addEventListener("click", () => void install());
  $("upd-later").addEventListener("click", () => void win.close());
}

function formatMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

void init();
