// Formatadores e ícones compartilhados entre a tela "Uso atual" (usage.ts) e o
// widget da área de trabalho (widget.ts). Funções puras, sem dependência de DOM.

export interface UsageMetric {
  ferramenta: string;
  uso_percentual: number;
  restante_percentual: number;
  status: string;
  coletado_em: string;
  reset_em: string | null;
  erro: string | null;
  uso_percentual_7d?: number;
  reset_em_7d?: string | null;
}
export interface ProviderUsage {
  habilitado: boolean;
  metric: UsageMetric | null;
}

// Ícones dos provedores no cabeçalho de cada card. Claude usa o "spark" laranja;
// o Codex usa o logo do Codex CLI (blob com gradiente roxo→azul e ">_" brancos),
// reproduzido como SVG inline.
export const ICON_CLAUDE = '<span class="spark">✳</span>';

// O logo do Codex usa um gradiente referenciado por `url(#id)`. Como o ícone
// aparece em várias telas que coexistem no DOM (e re-renderiza), um id FIXO
// colidiria no documento: o WebView resolve `url(#id)` para o primeiro elemento
// com aquele id e, quando esse é removido/re-renderizado, as outras instâncias
// ficam sem preenchimento (o "fundo roxo" some). Por isso cada chamada gera um
// id único.
let codexIconSeq = 0;
export function iconCodex(): string {
  const id = "codexg" + ++codexIconSeq;
  return (
    '<svg class="pico" width="20" height="20" viewBox="0 0 100 100" aria-hidden="true">' +
      '<defs><linearGradient id="' + id + '" x1="22" y1="6" x2="80" y2="98" gradientUnits="userSpaceOnUse">' +
        '<stop offset="0" stop-color="#a48cf2"/><stop offset=".55" stop-color="#5b62ec"/>' +
        '<stop offset="1" stop-color="#3a3fe6"/></linearGradient></defs>' +
      '<g fill="url(#' + id + ')">' +
        '<circle cx="50" cy="50" r="30"/><circle cx="50" cy="24" r="15"/><circle cx="68" cy="32" r="15"/>' +
        '<circle cx="76" cy="50" r="15"/><circle cx="68" cy="68" r="15"/><circle cx="50" cy="76" r="15"/>' +
        '<circle cx="32" cy="68" r="15"/><circle cx="24" cy="50" r="15"/><circle cx="32" cy="32" r="15"/>' +
      '</g>' +
      '<path d="M40 36 L53 50 L40 64" fill="none" stroke="#fff" stroke-width="9" ' +
        'stroke-linecap="round" stroke-linejoin="round"/>' +
      '<rect x="55" y="57" width="19" height="8" rx="4" fill="#fff"/>' +
    '</svg>'
  );
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string);
}

/// % sem casas quando inteiro, com 1 casa quando fracionário.
export function pctText(n: number): string {
  return Number.isInteger(n) ? String(n) : n.toFixed(1);
}

/// Cor da barra conforme o nível de uso: verde < 50% < amarelo < 80% < vermelho.
export function barColor(pct: number): string {
  if (pct >= 80) return "#e0816f";
  if (pct >= 50) return "#d9b35a";
  return "#7fc99a";
}

/// Tempo restante até o reset, humanizado: "5d 2h", "3h 33min", "12min", "40s".
export function fmtRemaining(iso: string): string {
  const ms = new Date(iso).getTime() - Date.now();
  if (Number.isNaN(ms)) return "—";
  if (ms <= 0) return "resetando…";
  const totalMin = Math.floor(ms / 60000);
  const days = Math.floor(totalMin / 1440);
  const hours = Math.floor((totalMin % 1440) / 60);
  const mins = totalMin % 60;
  if (days >= 1) return `${days}d ${hours}h`;
  if (hours >= 1) return `${hours}h ${mins}min`;
  if (totalMin >= 1) return `${mins}min`;
  return `${Math.floor(ms / 1000)}s`;
}

/// Data/hora exata do reset, no fuso local: "qui., 19/06, 17:00".
export function fmtExact(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleString("pt-BR", {
    weekday: "short", day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit",
  });
}

/// Só o horário do reset, no fuso local: "14:29".
export function fmtTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}

/// Hora/data exata do reset, compacta: "14:29" se for hoje, ou "22/06, 19:59"
/// se for outro dia (mesma regra da barra de tarefas).
export function fmtResetClock(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return sameDay
    ? d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString("pt-BR", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
}