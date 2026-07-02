// Login do Codex pelo navegador (OAuth 2.0 com PKCE), alternativa ao caminho do
// `auth.json`. E' uma porta do fluxo do Codex CLI: sobe um servidor local em
// 127.0.0.1:1455 para receber o callback, abre o navegador do sistema na tela de
// login da OpenAI e troca o `authorization code` por tokens (access/refresh/id).
//
// Os tokens sao gravados em um arquivo PROPRIO do app (`codex-auth.json`, no
// `config_dir`), no MESMO formato do `~/.codex/auth.json` — assim os leitores que
// ja existem (coleta de uso em `lib.rs` e dashboard em `codex_dashboard.rs`)
// funcionam sem alteracao, bastando apontar para este arquivo. NAO tocamos no
// `~/.codex/auth.json` do usuario.
//
// O `access_token` expira; guardamos `expires_at` e o renovamos via `refresh_token`
// em `ensure_fresh`, chamada antes de cada coleta no modo "navegador". O
// `account_id` (necessario ao dashboard) e o e-mail (so' para exibir "Conectado
// como ...") sao extraidos das claims do `id_token` (JWT).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Response, Server};

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const PORT: u16 = 1455;
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";
/// Tempo maximo aguardando o usuario concluir o login no navegador.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Renova o `access_token` com esta antecedencia da expiracao (folga p/ relogio/rede).
const REFRESH_SKEW_SECS: i64 = 120;

/// Pagina simples exibida no navegador ao concluir o login com sucesso.
const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html lang="pt-BR"><head><meta charset="UTF-8"><title>Autenticação concluída</title>
<style>body{font-family:system-ui,sans-serif;background:#0f1115;color:#e6e6e6;display:flex;
min-height:100vh;align-items:center;justify-content:center;margin:0}
.card{text-align:center;padding:2rem 2.5rem;background:#171a21;border-radius:14px;
box-shadow:0 10px 40px rgba(0,0,0,.4)}h1{font-size:1.25rem;margin:0 0 .5rem}
p{margin:0;color:#9aa4b2}</style></head>
<body><div class="card"><h1>Autenticação concluída ✓</h1>
<p>Você já pode fechar esta janela e voltar ao aplicativo.</p></div></body></html>"#;

/// Tokens persistidos no arquivo gerenciado. Espelha o formato do
/// `~/.codex/auth.json` (`tokens.*`, `last_refresh`) e acrescenta campos que so'
/// este app usa (`expires_at`, `email`); leitores externos ignoram os extras.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredAuth {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: StoredTokens,
    last_refresh: Option<String>,
    /// Instante de expiracao do `access_token`, em ms desde a epoch (UTC). Ausente
    /// = tratado como "renovar na proxima coleta".
    expires_at: Option<i64>,
    /// E-mail extraido do `id_token`, so' para exibir "Conectado como ...".
    email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredTokens {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    account_id: Option<String>,
}

/// Resposta do endpoint de token da OpenAI (login e refresh).
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<i64>,
}

/// Claims relevantes do `id_token` (JWT). O `account_id` do ChatGPT vem dentro do
/// namespace `https://api.openai.com/auth`.
#[derive(Debug, Deserialize)]
struct IdClaims {
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<AuthClaim>,
}

#[derive(Debug, Deserialize)]
struct AuthClaim {
    chatgpt_account_id: Option<String>,
}

/// Caminho do arquivo gerenciado de credenciais do Codex (login pelo navegador).
pub fn auth_file(config_dir: &Path) -> PathBuf {
    config_dir.join("codex-auth.json")
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn base64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Gera `len` bytes aleatorios e os codifica em base64url (para o `state` e o
/// `code_verifier` do PKCE).
fn random_base64url(len: usize) -> Result<String, String> {
    let mut buf = vec![0u8; len];
    getrandom::getrandom(&mut buf).map_err(|error| format!("Falha ao gerar aleatoriedade: {error}"))?;
    Ok(base64url(&buf))
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = url; // outras plataformas: sem abertura automatica de navegador.
    }
}

/// Decodifica o segmento de payload do `id_token` (JWT) e le' as claims. Retorna
/// `None` se o token nao for um JWT valido/decodificavel.
fn decode_id_claims(id_token: &str) -> Option<IdClaims> {
    let payload = id_token.split('.').nth(1)?;
    // JWTs usam base64url sem padding, mas toleramos padding removendo os '='.
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Percent-decode simples (para valores da query do callback, ex.: `error_description`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(key), percent_decode(value));
    }
    map
}

fn respond_text(request: tiny_http::Request, status: u16, text: &str) {
    let _ = request.respond(Response::from_string(text).with_status_code(status));
}

fn respond_html(request: tiny_http::Request, html: &str) {
    let mut response = Response::from_string(html).with_status_code(200);
    if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]) {
        response = response.with_header(header);
    }
    let _ = request.respond(response);
}

/// Sobe o servidor local e aguarda (ate' `LOGIN_TIMEOUT`) o callback do OAuth,
/// validando o `state`. Devolve o `authorization code`.
fn wait_for_code(state: &str) -> Result<String, String> {
    let server = Server::http(("127.0.0.1", PORT)).map_err(|error| {
        format!("Nao foi possivel abrir a porta {PORT} para o login (ja em uso?): {error}")
    })?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Tempo limite aguardando o login no navegador.".to_string())?;

        let request = match server.recv_timeout(remaining) {
            Ok(Some(request)) => request,
            Ok(None) => return Err("Tempo limite aguardando o login no navegador.".to_string()),
            Err(error) => return Err(format!("Falha no servidor de callback: {error}")),
        };

        let raw_url = request.url().to_string();
        let (path, query) = raw_url.split_once('?').unwrap_or((raw_url.as_str(), ""));
        if path != "/auth/callback" {
            respond_text(request, 404, "Not found");
            continue;
        }

        let params = parse_query(query);
        if params.get("state").map(String::as_str) != Some(state) {
            respond_text(request, 400, "OAuth state invalido.");
            return Err("OAuth state invalido.".to_string());
        }
        if let Some(oauth_error) = params.get("error") {
            let description = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| oauth_error.clone());
            respond_text(request, 400, &format!("Falha na autenticacao: {description}"));
            return Err(format!("Erro OAuth: {description}"));
        }
        match params.get("code") {
            Some(code) if !code.is_empty() => {
                let code = code.clone();
                respond_html(request, SUCCESS_HTML);
                return Ok(code);
            }
            _ => {
                respond_text(request, 400, "Authorization code nao retornado.");
                return Err("Authorization code nao retornado.".to_string());
            }
        }
    }
}

/// Troca o `authorization code` por tokens (grant_type=authorization_code + PKCE).
fn exchange_code(client: &Client, code: &str, verifier: &str) -> Result<TokenResponse, String> {
    post_token(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ],
        "trocar o código por tokens",
    )
}

/// Renova os tokens usando o `refresh_token` (grant_type=refresh_token).
fn exchange_refresh(client: &Client, refresh_token: &str) -> Result<TokenResponse, String> {
    post_token(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
            ("scope", SCOPES),
        ],
        "renovar o token do Codex",
    )
}

fn post_token(client: &Client, form: &[(&str, &str)], acao: &str) -> Result<TokenResponse, String> {
    let response = client
        .post(format!("{ISSUER}/oauth/token"))
        .form(form)
        .send()
        .map_err(|error| format!("Falha HTTP ao {acao}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("A OpenAI recusou a requisição ao {acao} (HTTP {status}): {body}"));
    }
    response
        .json::<TokenResponse>()
        .map_err(|error| format!("Resposta de token inválida ao {acao}: {error}"))
}

/// Monta o `StoredAuth` a partir da resposta de token. Na renovacao, alguns campos
/// podem nao voltar (ex.: `refresh_token`, `id_token`); nesse caso reaproveita os
/// valores anteriores (`previous`).
fn stored_from_tokens(
    tokens: TokenResponse,
    previous: Option<&StoredAuth>,
) -> Result<StoredAuth, String> {
    let access_token = tokens
        .access_token
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "A OpenAI não retornou access_token.".to_string())?;

    let refresh_token = tokens
        .refresh_token
        .filter(|token| !token.is_empty())
        .or_else(|| previous.and_then(|prev| prev.tokens.refresh_token.clone()));

    let id_token = tokens
        .id_token
        .filter(|token| !token.is_empty())
        .or_else(|| previous.and_then(|prev| prev.tokens.id_token.clone()));

    let claims = id_token.as_deref().and_then(decode_id_claims);
    let account_id = claims
        .as_ref()
        .and_then(|claims| claims.auth.as_ref())
        .and_then(|auth| auth.chatgpt_account_id.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| previous.and_then(|prev| prev.tokens.account_id.clone()));
    let email = claims
        .and_then(|claims| claims.email)
        .or_else(|| previous.and_then(|prev| prev.email.clone()));

    let expires_at = tokens.expires_in.map(|seconds| now_ms() + seconds * 1000);

    Ok(StoredAuth {
        openai_api_key: None,
        tokens: StoredTokens {
            id_token,
            access_token: Some(access_token),
            refresh_token,
            account_id,
        },
        last_refresh: Some(Utc::now().to_rfc3339()),
        expires_at,
        email,
    })
}

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
        .map_err(|error| format!("Falha ao gravar codex-auth.json: {error}"))
}

fn has_access_token(auth: &StoredAuth) -> bool {
    auth.tokens
        .access_token
        .as_deref()
        .map(|token| !token.is_empty())
        .unwrap_or(false)
}

/// Status do login pelo navegador, para a aba Codex das Configuracoes.
fn status_value(auth: &StoredAuth) -> Value {
    json!({
        "connected": has_access_token(auth),
        "email": auth.email,
        "expiresAt": auth.expires_at,
        "accountId": auth.tokens.account_id,
    })
}

// ---- API publica (consumida por lib.rs) ------------------------------------

/// Executa o login pelo navegador (BLOQUEANTE: sobe o servidor de callback, abre o
/// navegador e aguarda ate' `LOGIN_TIMEOUT`). Grava os tokens no arquivo gerenciado
/// e devolve o status (`{connected,email,expiresAt,accountId}`). Deve ser chamada
/// fora da main thread (ex.: `spawn_blocking`).
pub fn login(client: &Client, config_dir: &Path) -> Result<Value, String> {
    let state = random_base64url(32)?;
    let verifier = random_base64url(64)?;
    let challenge = base64url(Sha256::digest(verifier.as_bytes()).as_slice());

    let auth_url = reqwest::Url::parse_with_params(
        &format!("{ISSUER}/oauth/authorize"),
        &[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPES),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
            ("state", state.as_str()),
        ],
    )
    .map_err(|error| format!("URL de autorização inválida: {error}"))?;

    open_browser(auth_url.as_str());

    let code = wait_for_code(&state)?;
    let tokens = exchange_code(client, &code, &verifier)?;
    let stored = stored_from_tokens(tokens, None)?;

    let path = auth_file(config_dir);
    write_stored(&path, &stored)?;
    Ok(status_value(&stored))
}

/// Le' o status atual do login pelo navegador (sem rede).
pub fn status(config_dir: &Path) -> Value {
    match read_stored(&auth_file(config_dir)) {
        Some(auth) => status_value(&auth),
        None => json!({ "connected": false, "email": Value::Null, "expiresAt": Value::Null }),
    }
}

/// Remove as credenciais do login pelo navegador ("desconectar"). Idempotente.
pub fn logout(config_dir: &Path) -> Result<(), String> {
    let path = auth_file(config_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Falha ao remover codex-auth.json: {error}")),
    }
}

/// Garante um `access_token` valido no arquivo gerenciado e devolve o caminho, para
/// os leitores (coleta/dashboard) o lerem como um `auth.json` normal. Renova via
/// `refresh_token` quando o token esta' perto de expirar. Erro se nao ha login.
pub fn ensure_fresh(client: &Client, config_dir: &Path) -> Result<PathBuf, String> {
    let path = auth_file(config_dir);
    let mut auth = read_stored(&path).filter(has_access_token).ok_or_else(|| {
        "Codex não conectado. Faça o login pelo navegador nas Configurações.".to_string()
    })?;

    let needs_refresh = match auth.expires_at {
        Some(expires_at) => now_ms() + REFRESH_SKEW_SECS * 1000 >= expires_at,
        None => true,
    };
    if needs_refresh {
        match auth.tokens.refresh_token.clone().filter(|token| !token.is_empty()) {
            Some(refresh_token) => {
                let tokens = exchange_refresh(client, &refresh_token)?;
                auth = stored_from_tokens(tokens, Some(&auth))?;
                write_stored(&path, &auth)?;
            }
            None => {
                // Sem refresh_token: so' da' pra reconectar. Se o token ainda nao
                // expirou de fato, seguimos com ele; caso contrario, pedimos login.
                let expired = auth.expires_at.map(|exp| now_ms() >= exp).unwrap_or(false);
                if expired {
                    return Err(
                        "Sessão do Codex expirada. Reconecte pelo login do navegador.".to_string(),
                    );
                }
            }
        }
    }

    Ok(path)
}
