// Helpers compartilhados entre os dois dashboards (Claude em dashboard.ts e
// Codex em codex-dashboard.ts): rótulos de data, posicionamento de tooltip e
// seleção do botão de período. Funções puras (sem estado de módulo) — o range
// personalizado de cada tela é passado por parâmetro (ver normalizeRange).

export const MONTHS = ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."];

// Rótulo do botão de período personalizado.
export const CUSTOM_LABEL = "Personalizado";

// Date → "YYYY-MM-DD" no fuso local.
export const dkey = (d: Date): string =>
  d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") + "-" + String(d.getDate()).padStart(2, "0");

// "YYYY-MM-DD" → "DD de <mês>".
export function dayLabel(dateStr: string): string {
  const d = new Date(dateStr + "T12:00:00");
  return d.getDate() + " de " + MONTHS[d.getMonth()];
}

// "YYYY-MM-DD" → "DD/MM/AA" (usado no rótulo do botão "Personalizado").
export function fmtShort(key: string): string {
  const [y, m, d] = key.split("-");
  return d + "/" + m + "/" + y.slice(2);
}

// Range personalizado normalizado (inverte se "de" > "até"); strings vazias =
// limite aberto.
export function normalizeRange(from: string, to: string): { from: string; to: string } {
  if (from && to && from > to) [from, to] = [to, from];
  return { from, to };
}

// Posiciona um tooltip fixo perto do cursor (x, y), mantendo-o dentro da
// viewport: à esquerda do cursor se estourar a direita, abaixo se estourar o topo.
export function placeFixed(node: HTMLElement, x: number, y: number): void {
  const r = node.getBoundingClientRect();
  let px = x + 14, py = y - r.height - 10;
  if (px + r.width > innerWidth - 8) px = x - r.width - 14;
  if (py < 8) py = y + 16;
  node.style.left = px + "px";
  node.style.top = py + "px";
}

// Marca como "on" o botão clicado dentro do grupo `sel`, desmarcando os irmãos.
// Escopa ao container do PRÓPRIO alvo (target.closest(sel)) para não mexer em
// botões de mesma classe em outras telas — ".tabs"/".ranges" existem no Dashboard
// Claude, no Codex (.codex-tabs/.codex-ranges) e nas Configurações (.settings-tabs),
// e um seletor global apagaria o "on" da aba ativa das outras telas.
export function setOn(sel: string, target: EventTarget | null): void {
  const el = target instanceof Element ? target : null;
  const container = el?.closest(sel);
  if (!container) return;
  container.querySelectorAll("button").forEach((b) => b.classList.toggle("on", b === el));
}
