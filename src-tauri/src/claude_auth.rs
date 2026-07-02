// Login do Claude pelo navegador, alternativa ao preenchimento manual de
// `organizationId` + `cookie`. Diferente do Codex (que tem OAuth/PKCE publico), a
// claude.ai autentica por COOKIE de sessao web (`sessionKey`, httpOnly). Entao o
// fluxo aqui e' outro: abrimos um webview do Tauri em `claude.ai/login`, o usuario
// entra normalmente e o app captura o cookie `sessionKey` do webview (via
// `Webview::cookies_for_url`, que devolve inclusive httpOnly) — essa parte, por
// depender do webview, vive em `lib.rs` (`claude_login`).
//
// Aqui ficam as partes SEM Tauri: buscar o `organization_id` (com o cookie
// capturado, GET `claude.ai/api/organizations`), gravar/ler o arquivo gerenciado
// (`claude-auth.json` no config_dir), montar o header de cookie para a coleta,
// status/logout e a flag de cancelamento do login.
//
// Limitacao herdada do modelo de sessao web: o `sessionKey` expira e NAO ha
// refresh_token — quando expira (a coleta passa a dar 401/403) o usuario precisa
// reconectar. Nao ha renovacao automatica como no Codex.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";

/// Credenciais persistidas do login pelo navegador do Claude. So' o app usa este
/// arquivo (nao e' um formato de terceiros como o `auth.json` do Codex).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredAuth {
    session_key: Option<String>,
    organization_id: Option<String>,
    /// E-mail exibido ("Conectado como ..."), quando disponivel.
    email: Option<String>,
    /// Momento da captura (RFC 3339), so' informativo.
    captured_at: Option<String>,
    /// Marca que a sessao web expirou/foi rejeitada (a coleta recebeu 401/403).
    /// Como nao ha refresh_token, a UI usa isto para oferecer "Reconectar". E'
    /// setada pela coleta e limpa por um novo login.
    needs_reconnect: Option<bool>,
}

/// Caminho do arquivo gerenciado de credenciais do Claude (login pelo navegador).
pub fn auth_file(config_dir: &Path) -> PathBuf {
    config_dir.join("claude-auth.json")
}

/// Header `cookie` para a coleta, a partir do `sessionKey` capturado.
pub fn cookie_header(session_key: &str) -> String {
    format!("sessionKey={session_key}")
}

// ---- Flag de cancelamento do login em andamento -----------------------------

fn login_cancel_flag() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static FLAG: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    FLAG.get_or_init(|| Mutex::new(None))
}

/// Registra o inicio de um login (cancela um anterior preso) e devolve a flag que a
/// captura deve checar para abortar.
pub fn begin_login() -> Arc<AtomicBool> {
    cancel();
    let flag = Arc::new(AtomicBool::new(false));
    *login_cancel_flag().lock().unwrap_or_else(|p| p.into_inner()) = Some(flag.clone());
    flag
}

/// Limpa a flag do login que terminou (se ainda for a atual).
pub fn end_login(flag: &Arc<AtomicBool>) {
    let mut slot = login_cancel_flag().lock().unwrap_or_else(|p| p.into_inner());
    if slot.as_ref().map(|f| Arc::ptr_eq(f, flag)).unwrap_or(false) {
        *slot = None;
    }
}

/// Sinaliza o cancelamento do login pelo navegador em andamento (se houver).
pub fn cancel() {
    if let Some(flag) = login_cancel_flag().lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
}

// ---- Persistencia ------------------------------------------------------------

fn read_stored(path: &Path) -> Option<StoredAuth> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_stored(path: &Path, auth: &StoredAuth) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Falha ao criar diretório de credenciais: {error}"))?;
    }
    let payload = serde_json::to_string_pretty(auth).map_err(|error| error.to_string())?;
    std::fs::write(path, format!("{payload}\n"))
        .map_err(|error| format!("Falha ao gravar claude-auth.json: {error}"))
}

fn has_session(auth: &StoredAuth) -> bool {
    auth.session_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false)
        && auth.organization_id.as_deref().map(|o| !o.is_empty()).unwrap_or(false)
}

// ---- Busca do organization_id ------------------------------------------------

#[derive(Debug, Deserialize)]
struct Organization {
    uuid: Option<String>,
    name: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

/// Uma org candidata (com capability "chat") a ser oferecida ao usuario quando a
/// conta tem mais de uma — a coleta de uso e' por org, e escolher a errada faz o
/// app reportar 0% (ex.: org pessoal antiga vs. org de time realmente usada).
#[derive(Debug, Clone, Serialize)]
pub struct OrgCandidate {
    pub uuid: String,
    pub name: Option<String>,
}

/// Lista as organizacoes com capability "chat" (as que aparecem em claude.ai e tem
/// uso coletavel), preservando a ordem devolvida pela API. Usada para escolher a org
/// no login pelo navegador.
pub fn fetch_chat_organizations(
    client: &Client,
    session_key: &str,
) -> Result<Vec<OrgCandidate>, String> {
    let response = client
        .get("https://claude.ai/api/organizations")
        .header("accept", "*/*")
        .header("cookie", cookie_header(session_key))
        .header("referer", "https://claude.ai/")
        .header("user-agent", USER_AGENT)
        .send()
        .map_err(|error| format!("Falha HTTP ao buscar organização do Claude: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Claude retornou HTTP {} ao buscar a organização.",
            response.status()
        ));
    }
    let orgs: Vec<Organization> = response
        .json()
        .map_err(|error| format!("Falha ao decodificar organizações do Claude: {error}"))?;

    let candidates: Vec<OrgCandidate> = orgs
        .into_iter()
        .filter(|org| org.capabilities.iter().any(|c| c == "chat"))
        .filter_map(|org| {
            org.uuid
                .filter(|u| !u.is_empty())
                .map(|uuid| OrgCandidate { uuid, name: org.name })
        })
        .collect();
    Ok(candidates)
}

// ---- Sessao pendente entre login e escolha da org ----------------------------

/// Guarda o `sessionKey` capturado enquanto o usuario escolhe a org (quando a conta
/// tem mais de uma com "chat"). So' fica preenchido nesse intervalo curto.
fn pending_session() -> &'static Mutex<Option<String>> {
    static PENDING: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

/// Registra o `sessionKey` capturado, aguardando a escolha da org pelo usuario.
pub fn set_pending_session(session_key: &str) {
    *pending_session().lock().unwrap_or_else(|p| p.into_inner()) = Some(session_key.to_string());
}

/// Consome o `sessionKey` pendente (uma vez), quando o usuario confirma a org.
pub fn take_pending_session() -> Option<String> {
    pending_session().lock().unwrap_or_else(|p| p.into_inner()).take()
}

// ---- API publica usada por lib.rs --------------------------------------------

/// Grava as credenciais capturadas (sessionKey + org) e devolve o status.
pub fn store(
    config_dir: &Path,
    session_key: &str,
    organization_id: &str,
    email: Option<String>,
) -> Result<Value, String> {
    let auth = StoredAuth {
        session_key: Some(session_key.to_string()),
        organization_id: Some(organization_id.to_string()),
        email,
        captured_at: Some(Utc::now().to_rfc3339()),
        needs_reconnect: Some(false),
    };
    write_stored(&auth_file(config_dir), &auth)?;
    Ok(status_value(&auth))
}

fn status_value(auth: &StoredAuth) -> Value {
    let connected = has_session(auth);
    json!({
        "connected": connected,
        "needsReconnect": connected && auth.needs_reconnect.unwrap_or(false),
        "email": auth.email,
        "organizationId": auth.organization_id,
        "capturedAt": auth.captured_at,
    })
}

/// Status do login pelo navegador do Claude (sem rede).
pub fn status(config_dir: &Path) -> Value {
    match read_stored(&auth_file(config_dir)) {
        Some(auth) => status_value(&auth),
        None => json!({ "connected": false, "needsReconnect": false, "email": Value::Null, "organizationId": Value::Null }),
    }
}

/// Marca (ou limpa) que a sessao precisa de reconexao — chamada pela coleta ao
/// receber 401/403 (marca) ou sucesso (limpa). So' grava quando o valor muda, para
/// nao reescrever o arquivo a cada ciclo de coleta.
pub fn set_needs_reconnect(config_dir: &Path, value: bool) {
    let path = auth_file(config_dir);
    let Some(mut auth) = read_stored(&path) else {
        return;
    };
    if auth.needs_reconnect.unwrap_or(false) == value {
        return;
    }
    auth.needs_reconnect = Some(value);
    let _ = write_stored(&path, &auth);
}

/// Procura recursivamente um campo de e-mail (`email`/`email_address`) na resposta
/// JSON. Usado pelo melhor-esforco de descobrir o e-mail da conta.
fn find_email(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                let key = key.to_lowercase();
                if (key == "email" || key == "email_address") && val.as_str().map(|s| s.contains('@')).unwrap_or(false) {
                    return val.as_str().map(str::to_string);
                }
                if let Some(found) = find_email(val) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_email),
        _ => None,
    }
}

/// Melhor esforco para descobrir o e-mail da conta (so' para exibir "Conectado como
/// ..."). Nao e' essencial: qualquer falha ou ausencia retorna None.
pub fn fetch_email(client: &Client, session_key: &str) -> Option<String> {
    for url in ["https://claude.ai/api/account", "https://claude.ai/api/organizations"] {
        let response = match client
            .get(url)
            .header("accept", "*/*")
            .header("cookie", cookie_header(session_key))
            .header("referer", "https://claude.ai/")
            .header("user-agent", USER_AGENT)
            .send()
        {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };
        if let Ok(value) = response.json::<Value>() {
            if let Some(email) = find_email(&value) {
                return Some(email);
            }
        }
    }
    None
}

/// Remove as credenciais do login pelo navegador ("Desconectar"). Idempotente.
pub fn logout(config_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(auth_file(config_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Falha ao remover claude-auth.json: {error}")),
    }
}

/// Credenciais para a coleta no modo "navegador": (header de cookie, organization_id).
/// Erro se nao ha login salvo.
pub fn credentials(config_dir: &Path) -> Result<(String, String), String> {
    let auth = read_stored(&auth_file(config_dir))
        .filter(has_session)
        .ok_or_else(|| "Claude não conectado. Faça o login pelo navegador nas Configurações.".to_string())?;
    let session_key = auth.session_key.unwrap_or_default();
    let organization_id = auth.organization_id.unwrap_or_default();
    Ok((cookie_header(&session_key), organization_id))
}
