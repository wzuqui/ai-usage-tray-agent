//! Widgets nativos que desenham o uso de IA dentro da barra de tarefas do
//! Windows, um elemento por provedor (Codex, Claude).
//!
//! Para cada provedor habilitado cria uma pequena janela e a torna filha da
//! `Shell_TrayWnd` (a janela da barra de tarefas) via Win32 `SetParent`,
//! desenhando duas linhas com GDI:
//!
//! ```text
//!         Claude
//! 20% (2:36h) | 50% (2d)
//! ```
//!
//! Cuidados: DPI, reposicionamento periodico, recriacao se o Explorer reiniciar,
//! cor de texto pela cor real da barra e fundo transparente por color-key.

#![cfg(target_os = "windows")]

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS_NAME: PCWSTR = w!("AiUsageTaskbarWidget");
const TIMER_ID: usize = 1;
const WM_APP_UPDATE: u32 = WM_APP + 1;

/// Tamanho padrao da fonte em pontos (Segoe UI, regular).
const FONT_POINT_DEFAULT: i32 = 9;

/// Provedores exibidos, na ordem da esquerda para a direita.
const SLOTS: [(&str, &str); 2] = [("codex", "Codex"), ("claude", "Claude")];
const SLOT_COUNT: usize = SLOTS.len();

struct ProviderState {
    key: &'static str,
    label: &'static str,
    enabled: bool,
    detail: String,
    /// HWND da janela (0 = inexistente). Guardado como isize porque HWND nao e' `Send`.
    hwnd: isize,
}

static STATE: OnceLock<Mutex<Vec<ProviderState>>> = OnceLock::new();
/// HWND da `Shell_TrayWnd` atual (para detectar reinicio do Explorer).
static TASKBAR_HWND: AtomicIsize = AtomicIsize::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
/// Deslocamento horizontal (px): negativo move para a esquerda, positivo para a direita.
static OFFSET: AtomicI32 = AtomicI32::new(0);
/// Lado da barra onde os widgets sao ancorados: `false` = direita (padrao),
/// `true` = esquerda (util com o menu Iniciar centralizado, deixando a ponta
/// esquerda livre). O calculo de "adivinhar" a posicao e' espelhado.
static SIDE_LEFT: AtomicBool = AtomicBool::new(false);
/// Tamanho da fonte em pontos (configuravel via `barraTarefas.tamanhoFonte`).
static FONT_POINT: AtomicI32 = AtomicI32::new(FONT_POINT_DEFAULT);
/// Cor da fonte como COLORREF (0x00BBGGRR) ou -1 = automatico (preto/branco
/// conforme a cor real da barra). Configuravel via `barraTarefas.corFonte`.
static FONT_COLOR: AtomicI32 = AtomicI32::new(-1);
/// Aliases dos callbacks boxados (evitam repetir o `dyn Fn` e satisfazem o
/// clippy::type_complexity).
type ActivateCallback = Box<dyn Fn() + Send + Sync>;
type MenuCommandCallback = Box<dyn Fn(&str) + Send + Sync>;
/// Callback acionado quando o usuario clica num widget (abrir a janela do app).
static ON_ACTIVATE: OnceLock<ActivateCallback> = OnceLock::new();
/// Callback acionado quando um item do menu de contexto (clique direito) e'
/// escolhido. Recebe o id do item (mesmos ids do menu do tray).
static ON_MENU_COMMAND: OnceLock<MenuCommandCallback> = OnceLock::new();
/// Estado de pausa atual, para rotular o item "Pausar/Retomar coleta" no menu.
static PAUSED: AtomicBool = AtomicBool::new(false);

// Ids dos itens do menu de contexto (clique direito). Mapeiam para os mesmos
// ids de acao do menu do tray (ver `handle_menu_event` no lib.rs).
const CMD_OPEN_APP: usize = 1;
const CMD_OPEN_CONFIG: usize = 2;
const CMD_OPEN_LOGS: usize = 3;
const CMD_SEND_NOW: usize = 4;
const CMD_TOGGLE_PAUSE: usize = 5;
const CMD_QUIT: usize = 6;

/// Converte o id numerico do menu Win32 no id de acao usado por `handle_menu_event`.
fn menu_command_id(cmd: usize) -> Option<&'static str> {
    match cmd {
        CMD_OPEN_APP => Some("open_app"),
        CMD_OPEN_CONFIG => Some("open_config"),
        CMD_OPEN_LOGS => Some("open_logs"),
        CMD_SEND_NOW => Some("send_now"),
        CMD_TOGGLE_PAUSE => Some("toggle_pause"),
        CMD_QUIT => Some("quit"),
        _ => None,
    }
}

fn state() -> &'static Mutex<Vec<ProviderState>> {
    STATE.get_or_init(|| {
        Mutex::new(
            SLOTS
                .iter()
                .map(|&(key, label)| ProviderState {
                    key,
                    label,
                    enabled: false,
                    detail: String::new(),
                    hwnd: 0,
                })
                .collect(),
        )
    })
}

/// Trava o estado recuperando de um eventual envenenamento do Mutex (panic
/// anterior com o lock seguro). Critico nos callbacks `extern "system"` (wnd_proc/
/// paint): um `unwrap()` que desse panic ali estaria fazendo unwind atraves da
/// fronteira FFI (comportamento indefinido). Recuperar o guard evita o panic.
fn lock_state() -> std::sync::MutexGuard<'static, Vec<ProviderState>> {
    state().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

thread_local! {
    /// Fonte cacheada por altura da barra (recriada quando a altura muda).
    static FONT_CACHE: RefCell<Option<(i32, HFONT)>> = const { RefCell::new(None) };
    /// Cor de fundo/color-key amostrada da barra, por provedor (CLR_INVALID = ainda nao amostrada).
    static KEY_COLORS: RefCell<[u32; SLOT_COUNT]> = const { RefCell::new([CLR_INVALID; SLOT_COUNT]) };
    /// Ultima borda conhecida de um widget vizinho (-1 = nenhuma). Para o lado
    /// direito guarda a borda esquerda do vizinho; para o esquerdo, a direita.
    static STICKY_STRIP: Cell<i32> = const { Cell::new(-1) };
    /// Quantos ticks seguidos o vizinho nao foi detectado.
    static STRIP_MISSING: Cell<i32> = const { Cell::new(0) };
    /// Ultimo lado configurado observado (-1 = nenhum), para resetar o sticky
    /// quando o lado muda (as bordas guardadas tem semantica diferente por lado).
    static LAST_SIDE: Cell<i32> = const { Cell::new(-1) };
}

/// Ticks (~segundos) que mantemos a ultima posicao do vizinho quando ele some
/// da deteccao — cobre o painel de configuracoes rapidas aberto, que faz alguns
/// widgets (ex.: monitores de rede) sumirem temporariamente.
const STRIP_GRACE_TICKS: i32 = 120;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

/// Inicia a thread dos widgets. Idempotente.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(run_thread);
}

/// Define o deslocamento horizontal (px) dos widgets na barra. Negativo move
/// para a esquerda, positivo para a direita (util para nao sobrepor toolbars).
pub fn set_offset(px: i32) {
    OFFSET.store(px, Ordering::Relaxed);
}

/// Define o lado da barra onde os widgets ficam: `true` = esquerda, `false` =
/// direita (padrao). A deteccao da posicao e' espelhada conforme o lado.
pub fn set_side(left: bool) {
    SIDE_LEFT.store(left, Ordering::Relaxed);
}

/// Define o tamanho da fonte em pontos. A fonte e o layout sao recalculados no
/// proximo ciclo (a cache de fonte e' por altura em pixels).
pub fn set_font_size(pt: i32) {
    FONT_POINT.store(pt.max(1), Ordering::Relaxed);
}

/// Define a cor da fonte. `Some((r, g, b))` fixa a cor; `None` volta ao modo
/// automatico (preto/branco conforme a cor real da barra).
pub fn set_font_color(rgb: Option<(u8, u8, u8)>) {
    let value = match rgb {
        Some((r, g, b)) => (r as i32) | ((g as i32) << 8) | ((b as i32) << 16),
        None => -1,
    };
    FONT_COLOR.store(value, Ordering::Relaxed);
}

/// Registra o callback chamado quando o usuario clica num widget (ex.: mostrar a
/// janela do app). So o primeiro registro vale.
pub fn set_on_activate<F: Fn() + Send + Sync + 'static>(callback: F) {
    let _ = ON_ACTIVATE.set(Box::new(callback));
}

/// Registra o callback chamado quando um item do menu de contexto (clique
/// direito no widget) e' escolhido. So o primeiro registro vale.
pub fn set_on_menu_command<F: Fn(&str) + Send + Sync + 'static>(callback: F) {
    let _ = ON_MENU_COMMAND.set(Box::new(callback));
}

/// Informa o estado de pausa atual (para o rotulo do item de menu).
pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

/// Cor da fonte configurada, ou `None` se estiver em modo automatico.
fn font_color_override() -> Option<COLORREF> {
    let value = FONT_COLOR.load(Ordering::Relaxed);
    if value < 0 {
        None
    } else {
        Some(COLORREF(value as u32))
    }
}

/// Atualiza um provedor (habilitado + linha de detalhe) e pede repintura.
///
/// A criacao/destruicao das janelas acontece na thread dos widgets (proximo
/// tick do timer); aqui apenas guardamos o estado e avisamos para repintar.
pub fn set_provider(key: &str, enabled: bool, detail: String) {
    let hwnd = {
        let mut slots = lock_state();
        match slots.iter_mut().find(|slot| slot.key == key) {
            Some(slot) => {
                slot.enabled = enabled;
                slot.detail = detail;
                slot.hwnd
            }
            None => 0,
        }
    };
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut c_void)),
                WM_APP_UPDATE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

fn run_thread() {
    unsafe {
        register_class();
        maintain();
        SetTimer(None, TIMER_ID, 1000, None);

        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0);
            // 0 = WM_QUIT, -1 = erro.
            if result.0 == 0 || result.0 == -1 {
                break;
            }
            // Timer de thread (msg.hwnd nulo): roda manutencao fora do WndProc.
            // Obs.: com hWnd=NULL o SetTimer gera um ID proprio, entao nao da
            // para filtrar por TIMER_ID; basta ser um WM_TIMER da thread.
            if msg.hwnd.0.is_null() && msg.message == WM_TIMER {
                maintain();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn module_handle() -> HINSTANCE {
    match GetModuleHandleW(PCWSTR::null()) {
        Ok(module) => HINSTANCE(module.0),
        Err(_) => HINSTANCE(core::ptr::null_mut()),
    }
}

unsafe fn register_class() {
    if CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: module_handle(),
        hIcon: HICON::default(),
        hCursor: HCURSOR::default(),
        hbrBackground: HBRUSH::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: CLASS_NAME,
    };
    RegisterClassW(&class);
}

/// Garante que cada provedor habilitado tenha sua janela criada, anexada a barra
/// correta e posicionada; destroi as janelas de provedores desabilitados.
unsafe fn maintain() {
    let taskbar = match FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
        Ok(handle) => handle,
        Err(_) => return,
    };

    // Explorer reiniciou? As janelas filhas morreram junto com a barra antiga.
    let previous = TASKBAR_HWND.swap(taskbar.0 as isize, Ordering::SeqCst);
    if previous != taskbar.0 as isize {
        let mut slots = lock_state();
        for slot in slots.iter_mut() {
            slot.hwnd = 0;
        }
    }

    // Snapshot do estado para nao chamar Win32 segurando o lock.
    let snapshot: Vec<(usize, bool, isize)> = {
        let slots = lock_state();
        slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (index, slot.enabled, slot.hwnd))
            .collect()
    };

    // Cria/destroi janelas conforme o estado habilitado.
    let mut updates: Vec<(usize, isize)> = Vec::new();
    for (index, enabled, hwnd) in &snapshot {
        let exists = *hwnd != 0 && IsWindow(Some(HWND(*hwnd as *mut c_void))).as_bool();
        if *enabled && !exists {
            if let Some(handle) = create_widget(taskbar, *index) {
                updates.push((*index, handle.0 as isize));
            }
        } else if !*enabled && *hwnd != 0 {
            if exists {
                let _ = DestroyWindow(HWND(*hwnd as *mut c_void));
            }
            updates.push((*index, 0));
        }
    }
    if !updates.is_empty() {
        let mut slots = lock_state();
        for (index, hwnd) in updates {
            slots[index].hwnd = hwnd;
        }
    }

    position_widgets(taskbar);
}

unsafe fn create_widget(taskbar: HWND, index: usize) -> Option<HWND> {
    // Janelas com parent em outro processo nao podem ser criadas direto como
    // WS_CHILD (CreateWindowEx retorna ERROR_ALREADY_EXISTS). Cria-se como
    // top-level WS_POPUP oculta e depois reparenta-se com SetParent.
    //
    // Sem WS_EX_TRANSPARENT: o widget recebe cliques (WM_LBUTTONUP) para abrir o
    // app. WS_EX_NOACTIVATE mantem o foco onde estava (nao "rouba" ativacao); os
    // pixels do color-key continuam deixando o clique passar, entao o alvo
    // clicavel e' efetivamente o texto.
    let result = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        CLASS_NAME,
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        10,
        10,
        None,
        None,
        Some(module_handle()),
        None,
    );
    let hwnd = result.ok()?;

    if SetParent(hwnd, Some(taskbar)).is_err() {
        let _ = DestroyWindow(hwnd);
        return None;
    }

    // Identifica qual provedor esta janela representa.
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, index as isize);
    // Converte para janela filha visivel e aplica a mudanca de estilo.
    SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_CHILD | WS_VISIBLE).0 as isize);
    let _ = SetWindowPos(
        hwnd,
        Some(HWND_TOP),
        0,
        0,
        10,
        10,
        SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    let _ = ShowWindow(hwnd, SW_SHOWNA);
    Some(hwnd)
}

/// Aplica a logica de "carencia" (sticky) ao limite detectado do vizinho: se ele
/// sumiu temporariamente da deteccao (ex.: painel de config. rapidas aberto),
/// mantem a ultima posicao por `STRIP_GRACE_TICKS` ticks. Reseta o estado quando
/// o lado configurado muda, pois a borda guardada tem semantica diferente.
fn sticky_boundary(detected: Option<i32>, side_left: bool) -> Option<i32> {
    let side = side_left as i32;
    if LAST_SIDE.get() != side {
        LAST_SIDE.set(side);
        STICKY_STRIP.set(-1);
        STRIP_MISSING.set(0);
    }
    match detected {
        Some(value) => {
            STICKY_STRIP.set(value);
            STRIP_MISSING.set(0);
            Some(value)
        }
        None => {
            let missing = STRIP_MISSING.get() + 1;
            STRIP_MISSING.set(missing);
            let last = STICKY_STRIP.get();
            if last >= 0 && missing < STRIP_GRACE_TICKS {
                Some(last)
            } else {
                STICKY_STRIP.set(-1);
                None
            }
        }
    }
}

/// Posiciona os widgets habilitados, empilhados horizontalmente. Por padrao
/// ancora a direita (a esquerda da bandeja ou de outros widgets embutidos na
/// faixa direita). Com `lado = "esquerda"` ancora a esquerda (a direita de
/// eventuais widgets na ponta esquerda), util com o menu Iniciar centralizado.
/// O `deslocamento` (negativo = esquerda, positivo = direita) vale nos dois.
unsafe fn position_widgets(taskbar: HWND) {
    let dpi = {
        let value = GetDpiForWindow(taskbar);
        if value == 0 {
            96
        } else {
            value
        }
    };
    let scale = dpi as f32 / 96.0;

    let mut taskbar_rect = RECT::default();
    if GetClientRect(taskbar, &mut taskbar_rect).is_err() {
        return;
    }
    let height = taskbar_rect.bottom - taskbar_rect.top;
    if height <= 0 {
        return;
    }

    // Lista de widgets habilitados, na ordem da esquerda para a direita.
    let layout: Vec<(usize, isize, String)> = {
        let slots = lock_state();
        slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.enabled && slot.hwnd != 0)
            .map(|(index, slot)| (index, slot.hwnd, slot.detail.clone()))
            .collect()
    };
    if layout.is_empty() {
        return;
    }

    let font = get_font(dpi);
    let gap = (3.0 * scale) as i32; // gap ate o vizinho
    let between = (6.0 * scale) as i32; // gap entre widgets
    let offset = OFFSET.load(Ordering::Relaxed);
    let side_left = SIDE_LEFT.load(Ordering::Relaxed);

    // Calcula (x, largura) de cada widget conforme o lado.
    let mut placements: Vec<(usize, isize, i32, i32)> = Vec::with_capacity(layout.len());

    // Janelas-filhas estreitas da barra (widgets de terceiros: monitores de rede,
    // botao de clima, etc.) em coordenadas de cliente. Usadas para ancorar o
    // widget colado ao *cluster* vizinho, caminhando por janelas contiguas.
    let widgets = strip_widgets(taskbar, scale);
    // Tolerancia de "encostado": vao maximo entre um widget e o proximo do cluster.
    let gap_tol = (24.0 * scale) as i32;
    let bar_width = taskbar_rect.right - taskbar_rect.left;

    if side_left {
        // Borda esquerda: a direita do cluster de widgets de terceiro colado na
        // ponta esquerda da barra; senao, a propria borda esquerda (cliente x = 0).
        let strip = sticky_boundary(
            cluster_edge_left(&widgets, taskbar_rect.left, gap_tol, bar_width),
            true,
        );
        let mut boundary = taskbar_rect.left;
        if let Some(right) = strip {
            if right > boundary {
                boundary = right;
            }
        }
        let mut x_left = boundary + gap + offset;
        // Posiciona da esquerda para a direita (o primeiro da lista fica mais a esquerda).
        for (index, hwnd, detail) in layout.iter() {
            let label = SLOTS[*index].1;
            let width = measure_text(label, font, scale).max(measure_text(detail, font, scale));
            let x = x_left.max(0);
            placements.push((*index, *hwnd, x, width));
            x_left = x + width + between;
        }
    } else {
        // Borda direita: a esquerda da bandeja e, caminhando dali para a esquerda,
        // a esquerda do cluster de widgets de terceiro colados a ela (monitores de
        // rede, etc.). Comeca na borda esquerda da bandeja (ou na ponta direita da
        // barra, se a bandeja nao for encontrada).
        let tray_left = FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), PCWSTR::null())
            .ok()
            .and_then(|tray| window_left_in_client(taskbar, tray));
        let base = tray_left.unwrap_or(taskbar_rect.right);
        let strip = sticky_boundary(
            cluster_edge_right(&widgets, base, gap_tol, bar_width),
            false,
        );

        let mut boundary = base;
        if let Some(left) = strip {
            if left > 0 && left < boundary {
                boundary = left;
            }
        }
        let mut x_right = boundary - gap + offset;
        // Posiciona da direita para a esquerda (o ultimo da lista fica mais a direita).
        for (index, hwnd, detail) in layout.iter().rev() {
            let label = SLOTS[*index].1;
            let width = measure_text(label, font, scale).max(measure_text(detail, font, scale));
            let x = (x_right - width).max(0);
            placements.push((*index, *hwnd, x, width));
            x_right = x - between;
        }
    }

    // Aplica posicao + cor de fundo (color-key) amostrada da barra atras de cada
    // widget, para o ClearType misturar contra a cor verdadeira (sem halo cinza).
    for (index, hwnd, x, width) in placements {
        let widget = HWND(hwnd as *mut c_void);
        let bar = sample_bar_color(widget).unwrap_or_else(|| compute_colors().1);
        KEY_COLORS.with(|cache| cache.borrow_mut()[index] = bar.0);
        let _ = SetLayeredWindowAttributes(widget, bar, 0, LWA_COLORKEY);
        let _ = SetWindowPos(widget, Some(HWND_TOP), x, 0, width, height, SWP_NOACTIVATE);
        let _ = InvalidateRect(Some(widget), None, true);
    }
}

/// Amostra a cor da barra de tarefas num ponto logo a esquerda do widget.
unsafe fn sample_bar_color(widget: HWND) -> Option<COLORREF> {
    let mut rect = RECT::default();
    if GetWindowRect(widget, &mut rect).is_err() {
        return None;
    }
    let x = rect.left - 8;
    let y = (rect.top + rect.bottom) / 2;
    let screen = GetDC(None);
    if screen.0.is_null() {
        return None;
    }
    let pixel = GetPixel(screen, x, y);
    ReleaseDC(None, screen);
    if pixel.0 == CLR_INVALID {
        None
    } else {
        Some(pixel)
    }
}

/// Borda esquerda de uma janela convertida para coordenadas de cliente da barra.
unsafe fn window_left_in_client(taskbar: HWND, window: HWND) -> Option<i32> {
    let mut rect = RECT::default();
    if GetWindowRect(window, &mut rect).is_err() {
        return None;
    }
    let mut point = POINT {
        x: rect.left,
        y: rect.top,
    };
    let _ = ScreenToClient(taskbar, &mut point);
    Some(point.x)
}

struct WidgetScan {
    taskbar: isize,
    width_limit: i32,
    /// Bordas (esquerda, direita) de cada janela estreita, em coordenadas de
    /// cliente da barra.
    rects: Vec<(i32, i32)>,
}

/// True se o nome da classe e' o de um dos nossos proprios widgets.
fn is_our_widget_class(name: &[u16]) -> bool {
    name.iter().copied().eq("AiUsageTaskbarWidget".encode_utf16())
}

unsafe extern "system" fn widget_scan_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = &mut *(lparam.0 as *mut WidgetScan);

    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    // Ignora nossas proprias janelas (senao ancorariamos ao lado de nos mesmos).
    let mut class_buffer = [0u16; 64];
    let class_len = GetClassNameW(hwnd, &mut class_buffer);
    if class_len > 0 && is_our_widget_class(&class_buffer[..class_len as usize]) {
        return BOOL(1);
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return BOOL(1);
    }
    let width = rect.right - rect.left;
    // So janelas estreitas: exclui os containers largos do sistema (area de
    // tarefas, lista centralizada de apps, etc.).
    if width <= 0 || width > context.width_limit {
        return BOOL(1);
    }
    let taskbar = HWND(context.taskbar as *mut c_void);
    let mut point = POINT {
        x: rect.left,
        y: rect.top,
    };
    let _ = ScreenToClient(taskbar, &mut point);
    context.rects.push((point.x, point.x + width));
    BOOL(1)
}

/// Coleta as janelas-filhas estreitas da barra (widgets de terceiros), em
/// coordenadas de cliente, como pares (borda esquerda, borda direita).
unsafe fn strip_widgets(taskbar: HWND, scale: f32) -> Vec<(i32, i32)> {
    let mut context = WidgetScan {
        taskbar: taskbar.0 as isize,
        width_limit: (500.0 * scale) as i32,
        rects: Vec::new(),
    };
    let _ = EnumChildWindows(
        Some(taskbar),
        Some(widget_scan_proc),
        LPARAM(&mut context as *mut WidgetScan as isize),
    );
    context.rects
}

/// Borda esquerda do *cluster* de widgets colados a `base` pela direita,
/// caminhando para a esquerda por janelas contiguas (vao <= `gap_tol`). Comeca
/// na bandeja e absorve cada widget que encosta no limite atual, estendendo-o ate
/// a borda esquerda do widget. Janelas soltas no meio da barra (ex.: posicoes
/// transitorias no boot, longe da bandeja) nao sao contiguas e sao ignoradas.
/// Retorna `None` se nada foi absorvido. Trava de seguranca: nunca ancora antes
/// da metade da barra, evitando colar o widget ao Iniciar centralizado.
fn cluster_edge_right(widgets: &[(i32, i32)], base: i32, gap_tol: i32, bar_width: i32) -> Option<i32> {
    let floor = bar_width / 2;
    let mut boundary = base;
    loop {
        let mut next = boundary;
        for &(left, right) in widgets {
            // Widget que encosta no limite atual pela esquerda e o estende.
            if left < next && right >= boundary - gap_tol {
                next = next.min(left);
            }
        }
        if next < boundary && next >= floor {
            boundary = next;
        } else {
            break;
        }
    }
    if boundary < base {
        Some(boundary)
    } else {
        None
    }
}

/// Espelho de [`cluster_edge_right`] para o lado esquerdo: borda direita do
/// cluster colado a `base` (ponta esquerda da barra) caminhando para a direita.
/// Trava de seguranca: nunca passa da metade da barra.
fn cluster_edge_left(widgets: &[(i32, i32)], base: i32, gap_tol: i32, bar_width: i32) -> Option<i32> {
    let ceil = bar_width / 2;
    let mut boundary = base;
    loop {
        let mut next = boundary;
        for &(left, right) in widgets {
            // Widget que encosta no limite atual pela direita e o estende.
            if right > next && left <= boundary + gap_tol {
                next = next.max(right);
            }
        }
        if next > boundary && next <= ceil {
            boundary = next;
        } else {
            break;
        }
    }
    if boundary > base {
        Some(boundary)
    } else {
        None
    }
}

/// Decide a cor do texto e a cor de fundo/color-key com base na cor real da
/// barra de tarefas. Retorna `(texto, fundo)`.
fn compute_colors() -> (COLORREF, COLORREF) {
    const THEMES: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
    const DWM: &str = "Software\\Microsoft\\Windows\\DWM";

    // Quando "mostrar cor de destaque na barra" esta ligado, a barra usa a
    // accent color do DWM; usamos a luminancia dela para escolher texto claro
    // ou escuro e como cor de mistura das bordas.
    let prevalence = unsafe { read_hkcu_dword(THEMES, "ColorPrevalence") }.unwrap_or(0);
    if prevalence != 0 {
        if let Some(accent) = unsafe { read_hkcu_dword(DWM, "AccentColor") } {
            // AccentColor e' 0xAABBGGRR; os 24 bits baixos ja sao 0x00BBGGRR (COLORREF).
            let bar = COLORREF(accent & 0x00FF_FFFF);
            let text = if luminance(bar) > 0.5 {
                rgb(0, 0, 0)
            } else {
                rgb(255, 255, 255)
            };
            return (text, bar);
        }
    }

    // Sem accent na barra: usa o tema claro/escuro do sistema.
    let light = unsafe { read_hkcu_dword(THEMES, "SystemUsesLightTheme") }
        .map(|value| value != 0)
        .unwrap_or(false);
    if light {
        (rgb(0, 0, 0), rgb(243, 243, 243))
    } else {
        (rgb(255, 255, 255), rgb(32, 32, 32))
    }
}

/// Luminancia perceptual aproximada (0.0 a 1.0) de uma COLORREF (0x00BBGGRR).
fn luminance(color: COLORREF) -> f32 {
    let r = (color.0 & 0xFF) as f32;
    let g = ((color.0 >> 8) & 0xFF) as f32;
    let b = ((color.0 >> 16) & 0xFF) as f32;
    (0.299 * r + 0.587 * g + 0.114 * b) / 255.0
}

/// Converte o tamanho em pontos para pixels conforme o DPI (pt * dpi / 72).
fn font_pixel_height(dpi: u32) -> i32 {
    let pt = FONT_POINT.load(Ordering::Relaxed);
    ((pt * dpi as i32) + 36) / 72
}

fn get_font(dpi: u32) -> HFONT {
    let px = font_pixel_height(dpi);
    FONT_CACHE.with(|cache| {
        let mut slot = cache.borrow_mut();
        if let Some((cached_px, font)) = *slot {
            if cached_px == px {
                return font;
            }
            unsafe {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }

        // Segoe UI 9pt, regular (FW_NORMAL).
        let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let font = unsafe {
            CreateFontW(
                -px,
                0,
                0,
                0,
                400, // FW_NORMAL
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                PCWSTR(face.as_ptr()),
            )
        };
        *slot = Some((px, font));
        font
    })
}

unsafe fn measure_text(text: &str, font: HFONT, scale: f32) -> i32 {
    let dc = CreateCompatibleDC(None);
    let old = SelectObject(dc, HGDIOBJ(font.0));
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    if !wide.is_empty() {
        let _ = GetTextExtentPoint32W(dc, wide.as_slice(), &mut size);
    }
    SelectObject(dc, old);
    let _ = DeleteDC(dc);
    size.cx + (8.0 * scale) as i32
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    if width > 0 && height > 0 {
        let index = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
        let (label, detail) = {
            let slots = lock_state();
            match slots.get(index) {
                Some(slot) => (slot.label.to_string(), slot.detail.clone()),
                None => (String::new(), String::new()),
            }
        };

        // Double buffering para evitar flicker.
        let mem = CreateCompatibleDC(Some(hdc));
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let old_bitmap = SelectObject(mem, HGDIOBJ(bitmap.0));

        // Cor de fundo/color-key amostrada da barra (mesma usada no color-key da
        // janela); o texto fica preto em barra clara e branco em barra escura.
        let key = {
            let stored = KEY_COLORS.with(|cache| cache.borrow().get(index).copied().unwrap_or(CLR_INVALID));
            if stored == CLR_INVALID {
                compute_colors().1
            } else {
                COLORREF(stored)
            }
        };
        // Cor da fonte: a configurada, ou automatica (preto em barra clara,
        // branco em barra escura). O ClearType mistura contra `key` (cor real da
        // barra), entao a cor escolhida fica nitida.
        let text = font_color_override().unwrap_or_else(|| {
            if luminance(key) > 0.5 {
                rgb(0, 0, 0)
            } else {
                rgb(255, 255, 255)
            }
        });
        let brush = CreateSolidBrush(key);
        let full = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        FillRect(mem, &full, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let dpi = {
            let value = GetDpiForWindow(hwnd);
            if value == 0 {
                96
            } else {
                value
            }
        };
        let font = get_font(dpi);
        let old_font = SelectObject(mem, HGDIOBJ(font.0));
        SetBkMode(mem, TRANSPARENT);
        SetTextColor(mem, text);

        // Empilha as duas linhas coladas (altura real da fonte) e centraliza o
        // bloco verticalmente na barra, em vez de dividir a altura em metades.
        let mut line_size = SIZE::default();
        let probe = [b'0' as u16];
        let _ = GetTextExtentPoint32W(mem, &probe, &mut line_size);
        let line_h = line_size.cy.max(1);
        let line_gap = dpi as i32 / 96; // ~1px: vao total ~8px entre as linhas
        let block_h = line_h * 2 + line_gap;
        let top = ((height - block_h) / 2).max(0);
        let mut rect_label = RECT {
            left: 0,
            top,
            right: width,
            bottom: top + line_h,
        };
        let mut rect_detail = RECT {
            left: 0,
            top: top + line_h + line_gap,
            right: width,
            bottom: top + block_h,
        };
        let mut label_w: Vec<u16> = label.encode_utf16().collect();
        let mut detail_w: Vec<u16> = detail.encode_utf16().collect();
        let format = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
        DrawTextW(mem, label_w.as_mut_slice(), &mut rect_label, format);
        DrawTextW(mem, detail_w.as_mut_slice(), &mut rect_detail, format);

        SelectObject(mem, old_font);
        let _ = BitBlt(hdc, 0, 0, width, height, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
    }

    let _ = EndPaint(hwnd, &ps);
}

unsafe fn read_hkcu_dword(subkey: &str, value_name: &str) -> Option<u32> {
    let subkey_w: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let value_w: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut data: u32 = 0;
    let mut size: u32 = 4;
    let error = RegGetValueW(
        HKEY_CURRENT_USER,
        PCWSTR(subkey_w.as_ptr()),
        PCWSTR(value_w.as_ptr()),
        RRF_RT_REG_DWORD,
        None,
        Some(&mut data as *mut u32 as *mut c_void),
        Some(&mut size),
    );
    if error.0 == 0 {
        Some(data)
    } else {
        None
    }
}

/// Mostra o menu de contexto (mesmos itens do tray) na posicao do cursor e
/// dispara o callback com o item escolhido. Cria uma janela-dona top-level
/// invisivel: `TrackPopupMenu` precisa de uma janela em foreground para fechar
/// ao clicar fora, e o widget e' uma janela-filha (nao pode ser foreground).
/// Tambem usada pelo widget da area de trabalho (clique direito), via lib.rs.
pub unsafe fn show_context_menu() {
    let menu = match CreatePopupMenu() {
        Ok(handle) => handle,
        Err(_) => return,
    };

    let paused = PAUSED.load(Ordering::Relaxed);
    let pause_label = if paused {
        w!("Retomar coleta")
    } else {
        w!("Pausar coleta")
    };
    let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN_APP, w!("Abrir"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN_CONFIG, w!("Abrir config.json"));
    let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN_LOGS, w!("Abrir pasta de logs"));
    let _ = AppendMenuW(menu, MF_STRING, CMD_SEND_NOW, w!("Enviar agora"));
    let _ = AppendMenuW(menu, MF_STRING, CMD_TOGGLE_PAUSE, pause_label);
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Sair"));

    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);

    // Janela-dona invisivel (1x1, transparente) so' para ancorar o menu.
    let owner = match CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOOLWINDOW,
        CLASS_NAME,
        PCWSTR::null(),
        WS_POPUP,
        point.x,
        point.y,
        1,
        1,
        None,
        None,
        Some(module_handle()),
        None,
    ) {
        Ok(handle) => handle,
        Err(_) => {
            let _ = DestroyMenu(menu);
            return;
        }
    };
    let _ = SetLayeredWindowAttributes(owner, COLORREF(0), 0, LWA_ALPHA);
    let _ = ShowWindow(owner, SW_SHOW);
    let _ = SetForegroundWindow(owner);

    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        Some(0),
        owner,
        None,
    );
    // Truque classico (KB135788) para o menu fechar corretamente.
    let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    let _ = DestroyWindow(owner);

    if let Some(id) = menu_command_id(chosen.0 as usize) {
        if let Some(callback) = ON_MENU_COMMAND.get() {
            callback(id);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_APP_UPDATE => {
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // Clique no widget: abre a janela do app (mesma acao do clique no tray).
            if let Some(callback) = ON_ACTIVATE.get() {
                callback();
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            // Clique direito: abre o menu do app (mesmos itens do tray).
            show_context_menu();
            LRESULT(0)
        }
        WM_SETCURSOR => {
            // Mantem a seta padrao sobre o widget. Sem isto, como a classe nao tem
            // cursor, o DefWindowProc repassa o WM_SETCURSOR para a barra (outro
            // processo) e o cursor pode virar ampulheta. Tratamos aqui e
            // retornamos TRUE para encerrar o processamento.
            if let Ok(cursor) = LoadCursorW(None, IDC_ARROW) {
                let _ = SetCursor(Some(cursor));
            }
            LRESULT(1)
        }
        WM_DESTROY => {
            let index = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
            let mut slots = lock_state();
            if let Some(slot) = slots.get_mut(index) {
                if slot.hwnd == hwnd.0 as isize {
                    slot.hwnd = 0;
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}