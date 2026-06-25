// Verificação de atualização compartilhada pela janela. Roda UMA vez ao abrir o
// app (disparada em `main.ts`), guarda o resultado em cache e reflete no badge
// "Atualização disponível" do item "Sobre" do menu lateral. A tela "Sobre"
// consome o cache (não re-verifica ao abrir); só o botão "Buscar atualizações"
// força uma nova verificação por aqui.
import { invoke } from "@tauri-apps/api/core";

export interface UpdateStatus {
  available: boolean;
  currentVersion: string;
  newVersion: string | null;
  error: string | null;
}

let cached: UpdateStatus | null = null;
let inflight: Promise<UpdateStatus> | null = null;

// Liga/desliga o badge no item "Sobre" do menu conforme há (ou não) update.
function applyBadge(status: UpdateStatus | null): void {
  const nav = document.querySelector<HTMLElement>('.nav-item[data-view="sobre"]');
  if (!nav) return;
  nav.classList.toggle("has-update", !!status?.available);
}

// Verifica no backend (chamada de rede), atualiza o cache e o badge. Erros viram
// um status sem update (e sem badge) — o botão "Buscar atualizações" reporta.
export function checkUpdateStatus(): Promise<UpdateStatus> {
  const p = invoke<UpdateStatus>("check_update_status")
    .catch(
      (): UpdateStatus => ({
        available: false,
        currentVersion: cached?.currentVersion ?? "",
        newVersion: null,
        error: "check-failed",
      }),
    )
    .then((st) => {
      cached = st;
      applyBadge(st);
      return st;
    });
  inflight = p;
  return p;
}

// Status já verificado: o cache, ou a verificação em andamento (sem disparar uma
// nova). `null` se nada foi iniciado ainda.
export function getUpdateStatus(): UpdateStatus | Promise<UpdateStatus> | null {
  return cached ?? inflight;
}
