use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

const TRAY_ID: &str = "main-tray";
const APP_NAME_WINDOWS: &str = "AiUsageTrayAgent";
#[cfg(target_os = "linux")]
const APP_NAME_LINUX: &str = "ai-usage-tray-agent";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    usuario: String,
    intervalo_segundos: u64,
    loki: LokiConfig,
    providers: ProvidersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LokiConfig {
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProvidersConfig {
    codex: CodexConfig,
    claude: ClaudeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexConfig {
    habilitado: bool,
    auth_json_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeConfig {
    habilitado: bool,
    organization_id: String,
    cookie: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageMetric {
    usuario: String,
    ferramenta: String,
    uso_percentual: f64,
    restante_percentual: f64,
    status: String,
    coletado_em: String,
    reset_em: Option<String>,
    erro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uso_percentual_7d: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restante_percentual_7d: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reset_em_7d: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimePaths {
    config_dir: PathBuf,
    config_file: PathBuf,
    logs_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct RuntimeSnapshot {
    paused: bool,
    last_error: Option<String>,
    last_successful_send_at: Option<String>,
    codex_metric: Option<UsageMetric>,
    claude_metric: Option<UsageMetric>,
}

struct SharedState {
    snapshot: Mutex<RuntimeSnapshot>,
    cycle_lock: Mutex<()>,
    stop: AtomicBool,
}

#[derive(Debug, Deserialize)]
struct OpenCodeAuth {
    openai: Option<OpenAiAccess>,
    tokens: Option<OpenAiTokens>,
}

#[derive(Debug, Deserialize)]
struct OpenAiAccess {
    access: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiTokens {
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsageResponse {
    rate_limit: Option<OpenAiRateLimit>,
}

#[derive(Debug, Deserialize)]
struct OpenAiRateLimit {
    primary_window: Option<OpenAiPrimaryWindow>,
    secondary_window: Option<OpenAiSecondaryWindow>,
}

#[derive(Debug, Deserialize)]
struct OpenAiPrimaryWindow {
    used_percent: Option<f64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiSecondaryWindow {
    used_percent: Option<f64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeFiveHour>,
    seven_day: Option<ClaudeSevenDay>,
}

#[derive(Debug, Deserialize)]
struct ClaudeFiveHour {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeSevenDay {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            usuario: String::new(),
            intervalo_segundos: 10,
            loki: LokiConfig {
                url: String::new(),
            },
            providers: ProvidersConfig {
                codex: CodexConfig {
                    habilitado: true,
                    auth_json_path: String::new(),
                },
                claude: ClaudeConfig {
                    habilitado: true,
                    organization_id: String::new(),
                    cookie: String::new(),
                },
            },
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            let paths = ensure_storage()?;
            let _ = load_or_create_config(&paths)?;

            let shared = Arc::new(SharedState {
                snapshot: Mutex::new(RuntimeSnapshot::default()),
                cycle_lock: Mutex::new(()),
                stop: AtomicBool::new(false),
            });

            app.manage(paths.clone());
            app.manage(shared.clone());

            create_tray(app)?;
            refresh_tray(app.handle(), &shared)?;
            start_worker(app.handle().clone(), paths.clone(), shared.clone());

            Ok(())
        })
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<Arc<SharedState>>() {
                    state.stop.store(true, Ordering::Relaxed);
                }
            }
        });
}

fn create_tray<R: Runtime>(app: &mut tauri::App<R>) -> tauri::Result<()> {
    let menu = build_tray_menu(app.handle(), &RuntimeSnapshot::default())?;

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("AiUsageTrayAgent")
        .build(app)?;

    Ok(())
}

fn build_tray_menu<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: &RuntimeSnapshot,
) -> tauri::Result<Menu<R>> {
    let status_text = if snapshot.paused {
        "Status: coleta pausada".to_string()
    } else if let Some(error) = &snapshot.last_error {
        format!("Status: erro - {}", truncate(error, 64))
    } else {
        "Status: coleta ativa".to_string()
    };

    let codex_text = format!("Codex: {}", metric_text(snapshot.codex_metric.as_ref()));
    let claude_text = format!("Claude: {}", metric_text(snapshot.claude_metric.as_ref()));
    let pause_label = if snapshot.paused {
        "Retomar coleta"
    } else {
        "Pausar coleta"
    };

    let status_item = MenuItem::with_id(app, "status", status_text, false, None::<&str>)?;
    let codex_item = MenuItem::with_id(app, "codex_status", codex_text, false, None::<&str>)?;
    let claude_item = MenuItem::with_id(app, "claude_status", claude_text, false, None::<&str>)?;
    let open_config_item =
        MenuItem::with_id(app, "open_config", "Abrir config.json", true, None::<&str>)?;
    let open_logs_item =
        MenuItem::with_id(app, "open_logs", "Abrir pasta de logs", true, None::<&str>)?;
    let send_now_item = MenuItem::with_id(app, "send_now", "Enviar agora", true, None::<&str>)?;
    let toggle_pause_item =
        MenuItem::with_id(app, "toggle_pause", pause_label, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[
            &status_item,
            &codex_item,
            &claude_item,
            &PredefinedMenuItem::separator(app)?,
            &open_config_item,
            &open_logs_item,
            &send_now_item,
            &toggle_pause_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, menu_id: &str) {
    match menu_id {
        "open_config" => {
            if let Some(paths) = app.try_state::<RuntimePaths>() {
                if let Err(error) = open_path(&paths.config_file) {
                    handle_runtime_error(app, &format!("Falha ao abrir config.json: {error}"));
                }
            }
        }
        "open_logs" => {
            if let Some(paths) = app.try_state::<RuntimePaths>() {
                if let Err(error) = open_path(&paths.logs_dir) {
                    handle_runtime_error(app, &format!("Falha ao abrir logs: {error}"));
                }
            }
        }
        "send_now" => {
            trigger_collection(app);
        }
        "toggle_pause" => {
            if let Some(shared) = app.try_state::<Arc<SharedState>>() {
                {
                    let mut snapshot = shared.snapshot.lock().unwrap();
                    snapshot.paused = !snapshot.paused;
                }

                let _ = append_log_line(
                    app.state::<RuntimePaths>().inner(),
                    "info",
                    "Coleta pausada/retomada pelo usuario.",
                    None,
                );
                let _ = refresh_tray(app, &shared);
            }
        }
        "quit" => {
            app.exit(0);
        }
        _ => {}
    }
}

fn trigger_collection<R: Runtime>(app: &AppHandle<R>) {
    let app_handle = app.clone();
    let paths = app.state::<RuntimePaths>().inner().clone();
    let shared = app.state::<Arc<SharedState>>().inner().clone();

    let _ = append_log_line(
        &paths,
        "info",
        "Envio manual solicitado pelo usuario.",
        None,
    );

    thread::spawn(move || {
        let _ = run_collection_cycle(&app_handle, &paths, &shared);
    });
}

fn start_worker<R: Runtime + 'static>(
    app: AppHandle<R>,
    paths: RuntimePaths,
    shared: Arc<SharedState>,
) {
    thread::spawn(move || loop {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }

        let interval = match load_or_create_config(&paths) {
            Ok(config) => config.intervalo_segundos.clamp(5, 3600),
            Err(error) => {
                handle_runtime_error(&app, &format!("Falha ao carregar config: {error}"));
                10
            }
        };

        let paused = shared.snapshot.lock().unwrap().paused;
        if !paused {
            let _ = run_collection_cycle(&app, &paths, &shared);
        } else {
            let _ = refresh_tray(&app, &shared);
        }

        for _ in 0..interval {
            if shared.stop.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn run_collection_cycle<R: Runtime>(
    app: &AppHandle<R>,
    paths: &RuntimePaths,
    shared: &Arc<SharedState>,
) -> Result<(), String> {
    let _lock = shared.cycle_lock.lock().unwrap();
    let config = load_or_create_config(paths).map_err(|error| error.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;

    let mut had_error = false;

    if config.providers.codex.habilitado {
        match collect_codex_metric(&client, &config) {
            Ok(metric) => {
                update_metric(shared, metric.clone());
                if let Err(error) = send_metric_to_loki(&client, &config, &metric) {
                    had_error = true;
                    let _ = append_log_line(
                        paths,
                        "error",
                        "Falha ao enviar metrica para o Loki.",
                        Some(json!({
                            "ferramenta": "codex",
                            "error": error
                        })),
                    );
                    handle_runtime_error(app, &error);
                } else {
                    let _ = append_log_line(
                        paths,
                        "info",
                        "Metrica enviada para o Loki.",
                        Some(json!({
                            "ferramenta": "codex",
                            "uso_percentual": metric.uso_percentual,
                            "uso_percentual_7d": metric.uso_percentual_7d,
                            "status": metric.status,
                            "reset_em": metric.reset_em,
                            "reset_em_7d": metric.reset_em_7d
                        })),
                    );
                    mark_success(shared);
                }
            }
            Err(error) => {
                had_error = true;
                let metric = build_error_metric(&config.usuario, "codex", &error);
                update_metric(shared, metric);
                let _ = append_log_line(
                    paths,
                    "error",
                    "Falha ao coletar metrica.",
                    Some(json!({
                        "ferramenta": "codex",
                        "error": error
                    })),
                );
                handle_runtime_error(app, &error);
            }
        }
    }

    if config.providers.claude.habilitado {
        match collect_claude_metric(&client, &config) {
            Ok(metric) => {
                update_metric(shared, metric.clone());
                if let Err(error) = send_metric_to_loki(&client, &config, &metric) {
                    had_error = true;
                    let _ = append_log_line(
                        paths,
                        "error",
                        "Falha ao enviar metrica para o Loki.",
                        Some(json!({
                            "ferramenta": "claude",
                            "error": error
                        })),
                    );
                    handle_runtime_error(app, &error);
                } else {
                    let _ = append_log_line(
                        paths,
                        "info",
                        "Metrica enviada para o Loki.",
                        Some(json!({
                            "ferramenta": "claude",
                            "uso_percentual": metric.uso_percentual,
                            "uso_percentual_7d": metric.uso_percentual_7d,
                            "status": metric.status,
                            "reset_em": metric.reset_em,
                            "reset_em_7d": metric.reset_em_7d
                        })),
                    );
                    mark_success(shared);
                }
            }
            Err(error) => {
                had_error = true;
                let metric = build_error_metric(&config.usuario, "claude", &error);
                update_metric(shared, metric);
                let _ = append_log_line(
                    paths,
                    "error",
                    "Falha ao coletar metrica.",
                    Some(json!({
                        "ferramenta": "claude",
                        "error": error
                    })),
                );
                handle_runtime_error(app, &error);
            }
        }
    }

    if !config.providers.codex.habilitado && !config.providers.claude.habilitado {
        handle_runtime_error(app, "Nenhum provider habilitado.");
    } else if !had_error {
        clear_last_error(shared);
    }

    refresh_tray(app, shared).map_err(|error| error.to_string())?;
    Ok(())
}

fn collect_codex_metric(client: &Client, config: &AppConfig) -> Result<UsageMetric, String> {
    let auth_path = config.providers.codex.auth_json_path.trim();
    if auth_path.is_empty() {
        return Err("Caminho do auth.json do Codex nao configurado.".to_string());
    }

    let auth_raw = fs::read_to_string(auth_path)
        .map_err(|error| format!("Falha ao ler auth.json do Codex: {error}"))?;
    let auth: OpenCodeAuth =
        serde_json::from_str(&auth_raw).map_err(|error| format!("auth.json invalido: {error}"))?;
    let token = auth
        .openai
        .and_then(|value| value.access)
        .or_else(|| auth.tokens.and_then(|value| value.access_token))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Campos openai.access ou tokens.access_token nao foram encontrados no auth.json do Codex."
                .to_string()
        })?;

    let response = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("accept", "*/*")
        .header("accept-language", "pt-BR,pt;q=0.9,en;q=0.8")
        .header("authorization", format!("Bearer {token}"))
        .header("cache-control", "no-cache")
        .header("pragma", "no-cache")
        .header("oai-language", "pt-BR")
        .header("x-openai-target-path", "/backend-api/wham/usage")
        .header("x-openai-target-route", "/backend-api/wham/usage")
        .send()
        .map_err(|error| format!("Falha HTTP ao consultar Codex: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Codex retornou status HTTP {}.",
            response.status()
        ));
    }

    let payload: OpenAiUsageResponse = response
        .json()
        .map_err(|error| format!("Falha ao decodificar resposta do Codex: {error}"))?;

    let rate_limit = payload
        .rate_limit
        .ok_or_else(|| "rate_limit nao foi encontrado na resposta do Codex.".to_string())?;

    let primary_window = rate_limit
        .primary_window
        .ok_or_else(|| "rate_limit.primary_window nao foi encontrado na resposta do Codex.".to_string())?;

    let used_percent = primary_window
        .used_percent
        .ok_or_else(|| {
            "rate_limit.primary_window.used_percent nao foi encontrado na resposta do Codex."
                .to_string()
        })?;

    let secondary_used_percent = rate_limit
        .secondary_window
        .as_ref()
        .and_then(|value| value.used_percent);
    let secondary_reset_at = rate_limit
        .secondary_window
        .as_ref()
        .and_then(|value| value.reset_at);

    Ok(UsageMetric {
        usuario: normalized_user(&config.usuario),
        ferramenta: "codex".to_string(),
        uso_percentual: round_percent(used_percent),
        restante_percentual: round_percent(100.0 - used_percent),
        status: "ok".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: primary_window.reset_at.and_then(timestamp_seconds_to_iso),
        erro: None,
        uso_percentual_7d: secondary_used_percent.map(round_percent),
        restante_percentual_7d: secondary_used_percent.map(|value| round_percent(100.0 - value)),
        reset_em_7d: secondary_reset_at.and_then(timestamp_seconds_to_iso),
    })
}

fn collect_claude_metric(client: &Client, config: &AppConfig) -> Result<UsageMetric, String> {
    let organization_id = config.providers.claude.organization_id.trim();
    let cookie = config.providers.claude.cookie.trim();

    if organization_id.is_empty() {
        return Err("Organization ID do Claude nao configurado.".to_string());
    }

    if cookie.is_empty() {
        return Err("Cookie do Claude nao configurado.".to_string());
    }

    let response = client
        .get(format!(
            "https://claude.ai/api/organizations/{organization_id}/usage"
        ))
        .header("accept", "*/*")
        .header("cookie", cookie)
        .header("referer", "https://claude.ai/settings/usage")
        .header(
            "user-agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36",
        )
        .send()
        .map_err(|error| format!("Falha HTTP ao consultar Claude: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Claude retornou status HTTP {}.",
            response.status()
        ));
    }

    let payload: ClaudeUsageResponse = response
        .json()
        .map_err(|error| format!("Falha ao decodificar resposta do Claude: {error}"))?;

    let five_hour = payload
        .five_hour
        .ok_or_else(|| "five_hour nao foi encontrado na resposta do Claude.".to_string())?;
    let utilization = five_hour
        .utilization
        .ok_or_else(|| "five_hour.utilization nao foi encontrado na resposta do Claude.".to_string())?;

    let seven_day_utilization = payload
        .seven_day
        .as_ref()
        .and_then(|value| value.utilization);
    let seven_day_resets_at = payload
        .seven_day
        .as_ref()
        .and_then(|value| value.resets_at.clone());

    Ok(UsageMetric {
        usuario: normalized_user(&config.usuario),
        ferramenta: "claude".to_string(),
        uso_percentual: round_percent(utilization),
        restante_percentual: round_percent(100.0 - utilization),
        status: "ok".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: five_hour.resets_at,
        erro: None,
        uso_percentual_7d: seven_day_utilization.map(round_percent),
        restante_percentual_7d: seven_day_utilization.map(|value| round_percent(100.0 - value)),
        reset_em_7d: seven_day_resets_at,
    })
}

fn send_metric_to_loki(client: &Client, config: &AppConfig, metric: &UsageMetric) -> Result<(), String> {
    if config.loki.url.trim().is_empty() {
        return Err("URL do Loki nao configurada.".to_string());
    }

    let timestamp_nanos = iso_to_nanos(&metric.coletado_em)?;
    let host = hostname::get()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut body = json!({
        "uso_percentual": metric.uso_percentual,
        "restante_percentual": metric.restante_percentual,
        "status": metric.status,
        "reset_em": metric.reset_em
    });

    if let Some(error) = &metric.erro {
        body["erro"] = Value::String(error.clone());
    }

    if let Some(value) = metric.uso_percentual_7d {
        body["uso_percentual_7d"] = json!(value);
    }
    if let Some(value) = metric.restante_percentual_7d {
        body["restante_percentual_7d"] = json!(value);
    }
    if let Some(value) = &metric.reset_em_7d {
        body["reset_em_7d"] = Value::String(value.clone());
    }

    let payload = json!({
        "streams": [
            {
                "stream": {
                    "app": "ai-usage-tray-agent",
                    "usuario": metric.usuario,
                    "ferramenta": metric.ferramenta,
                    "host": host
                },
                "values": [
                    [timestamp_nanos, body.to_string()]
                ]
            }
        ]
    });

    let request = client
        .post(config.loki.url.trim())
        .header("content-type", "application/json")
        .json(&payload);

    let response = request
        .send()
        .map_err(|error| format!("Falha HTTP ao enviar para Loki: {error}"))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Loki retornou status HTTP {}.", response.status()))
    }
}

fn refresh_tray<R: Runtime>(app: &AppHandle<R>, shared: &Arc<SharedState>) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let snapshot = shared.snapshot.lock().unwrap().clone();
        let tooltip = format!(
            "AiUsageTrayAgent\nCodex: {}\nClaude: {}",
            metric_tooltip_text(snapshot.codex_metric.as_ref()),
            metric_tooltip_text(snapshot.claude_metric.as_ref())
        );

        #[cfg(target_os = "windows")]
        tray.set_tooltip(Some(tooltip))?;

        #[cfg(target_os = "linux")]
        tray.set_title(Some(format!(
            "C:{} / Cl:{}",
            metric_text(snapshot.codex_metric.as_ref()),
            metric_text(snapshot.claude_metric.as_ref())
        )))?;

        let menu = build_tray_menu(app, &snapshot)?;
        tray.set_menu(Some(menu))?;
    }

    Ok(())
}

fn update_metric(shared: &Arc<SharedState>, metric: UsageMetric) {
    let mut snapshot = shared.snapshot.lock().unwrap();
    match metric.ferramenta.as_str() {
        "codex" => snapshot.codex_metric = Some(metric),
        "claude" => snapshot.claude_metric = Some(metric),
        _ => {}
    }
}

fn mark_success(shared: &Arc<SharedState>) {
    let mut snapshot = shared.snapshot.lock().unwrap();
    snapshot.last_successful_send_at = Some(Utc::now().to_rfc3339());
}

fn clear_last_error(shared: &Arc<SharedState>) {
    let mut snapshot = shared.snapshot.lock().unwrap();
    snapshot.last_error = None;
}

fn handle_runtime_error<R: Runtime>(app: &AppHandle<R>, message: &str) {
    if let Some(shared) = app.try_state::<Arc<SharedState>>() {
        let mut snapshot = shared.snapshot.lock().unwrap();
        snapshot.last_error = Some(message.to_string());
    }

    let _ = append_log_line(
        app.state::<RuntimePaths>().inner(),
        "error",
        message,
        None,
    );

    if let Some(shared) = app.try_state::<Arc<SharedState>>() {
        let _ = refresh_tray(app, &shared);
    }
}

fn build_error_metric(usuario: &str, ferramenta: &str, erro: &str) -> UsageMetric {
    UsageMetric {
        usuario: normalized_user(usuario),
        ferramenta: ferramenta.to_string(),
        uso_percentual: 0.0,
        restante_percentual: 100.0,
        status: "erro".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: None,
        erro: Some(erro.to_string()),
        uso_percentual_7d: None,
        restante_percentual_7d: None,
        reset_em_7d: None,
    }
}

fn normalized_user(usuario: &str) -> String {
    let trimmed = usuario.trim();
    if trimmed.is_empty() {
        "desconhecido".to_string()
    } else {
        trimmed.to_string()
    }
}

fn metric_text(metric: Option<&UsageMetric>) -> String {
    let Some(metric) = metric else {
        return "--".to_string();
    };
    let session = format!("{:.1}%", metric.uso_percentual);
    match metric.uso_percentual_7d {
        Some(seven_day) => format!("{session} | {:.1}% (7d)", seven_day),
        None => session,
    }
}

fn metric_tooltip_text(metric: Option<&UsageMetric>) -> String {
    let Some(metric) = metric else {
        return "--".to_string();
    };
    let session = format!("{:.1}%", metric.uso_percentual);
    match metric.reset_em.as_deref().and_then(format_time_until) {
        Some(remaining) => format!("{session} ({remaining})"),
        None => session,
    }
}

fn format_time_until(reset_iso: &str) -> Option<String> {
    let reset = DateTime::parse_from_rfc3339(reset_iso).ok()?;
    let delta = reset.with_timezone(&Utc).signed_duration_since(Utc::now());
    let total_secs = delta.num_seconds();
    if total_secs <= 0 {
        return Some("agora".to_string());
    }
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    if days > 0 {
        Some(format!("{days}d{hours}h"))
    } else if hours > 0 {
        Some(format!("{hours}h{minutes}m"))
    } else {
        Some(format!("{minutes}m"))
    }
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }

    text.chars().take(max_len.saturating_sub(3)).collect::<String>() + "..."
}

fn iso_to_nanos(iso: &str) -> Result<String, String> {
    let timestamp = DateTime::parse_from_rfc3339(iso)
        .map_err(|error| format!("Timestamp invalido para Loki: {error}"))?;
    Ok(timestamp.timestamp_nanos_opt().unwrap_or_default().to_string())
}

fn timestamp_seconds_to_iso(value: i64) -> Option<String> {
    DateTime::from_timestamp(value, 0).map(|timestamp| timestamp.to_rfc3339())
}

fn round_percent(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn ensure_storage() -> Result<RuntimePaths, Box<dyn std::error::Error>> {
    let paths = runtime_paths()?;
    fs::create_dir_all(&paths.config_dir)?;
    fs::create_dir_all(&paths.logs_dir)?;
    Ok(paths)
}

fn runtime_paths() -> Result<RuntimePaths, Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    {
        let app_data = env::var("APPDATA")
            .map(PathBuf::from)
            .or_else(|_| dirs::config_dir().ok_or(env::VarError::NotPresent))?;
        let local_app_data = env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|_| dirs::data_local_dir().ok_or(env::VarError::NotPresent))?;

        return Ok(RuntimePaths {
            config_dir: app_data.join(APP_NAME_WINDOWS),
            config_file: app_data.join(APP_NAME_WINDOWS).join("config.json"),
            logs_dir: local_app_data.join(APP_NAME_WINDOWS).join("logs"),
        });
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("Home directory nao encontrada.")?;

        return Ok(RuntimePaths {
            config_dir: home.join(".config").join(APP_NAME_LINUX),
            config_file: home
                .join(".config")
                .join(APP_NAME_LINUX)
                .join("config.json"),
            logs_dir: home.join(".local").join("state").join(APP_NAME_LINUX).join("logs"),
        });
    }

    #[allow(unreachable_code)]
    Err("Sistema operacional nao suportado.".into())
}

fn load_or_create_config(paths: &RuntimePaths) -> Result<AppConfig, Box<dyn std::error::Error>> {
    if !paths.config_file.exists() {
        let default_config = AppConfig::default();
        let payload = serde_json::to_string_pretty(&default_config)?;
        fs::write(&paths.config_file, format!("{payload}\n"))?;
        return Ok(default_config);
    }

    let content = fs::read_to_string(&paths.config_file)?;
    let mut config: AppConfig = serde_json::from_str(&content)?;
    config.intervalo_segundos = config.intervalo_segundos.clamp(5, 3600);
    Ok(config)
}

fn append_log_line(
    paths: &RuntimePaths,
    level: &str,
    message: &str,
    meta: Option<Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(&paths.logs_dir)?;

    let log_path = paths
        .logs_dir
        .join(format!("{}.log", Utc::now().format("%Y-%m-%d")));
    let mut file = open_append_file(&log_path)?;
    let payload = json!({
        "timestamp": Utc::now().to_rfc3339(),
        "level": level,
        "message": message,
        "meta": meta
    });

    writeln!(file, "{payload}")?;
    Ok(())
}

fn open_append_file(path: &Path) -> Result<File, Box<dyn std::error::Error>> {
    Ok(OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?)
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Abertura de caminho nao suportada neste sistema.".to_string())
}
