// Dashboard de uso do Codex, alimentado pela API de analytics do backend do
// ChatGPT (namespace `wham`). Diferente do dashboard do Claude (que parseia
// arquivos locais), aqui os dados vêm de uma chamada de rede leve usando o
// mesmo `access_token` do `~/.codex/auth.json` que a coleta de uso já usa.
//
//   GET /backend-api/wham/usage/daily-token-usage-breakdown
//        ?start_date=YYYY-MM-DD&end_date=YYYY-MM-DD&group_by=day
//   -> { data: [{ date, product_surface_usage_values, models[] }], units, group_by }
//
// Auth: `Authorization: Bearer <tokens.access_token>` + `chatgpt-account-id:
// <tokens.account_id>`. O `access_token` expira (~10 dias); reler o arquivo a
// cada chamada pega o token renovado pelo app do Codex.
//
// Os dados são expostos ao frontend pelo comando Tauri `get_codex_stats` (IPC),
// definido em lib.rs, que delega para `collect` aqui. A unidade é PERCENTUAL de
// uso diário (não tokens absolutos), então a UI rotula o eixo como "% de uso".

use chrono::{Duration, Local, NaiveDate};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;

#[derive(Debug, Deserialize)]
struct CodexAuth {
    tokens: Option<AuthTokens>,
    openai: Option<AuthOpenAi>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthOpenAi {
    access: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BreakdownResponse {
    #[serde(default)]
    data: Vec<Value>,
    units: Option<String>,
    group_by: Option<String>,
}

/// Busca o histórico diário de uso do Codex e devolve um `Value` pronto para o
/// frontend. Em qualquer falha (sem caminho, token ausente/expirado, rede,
/// status HTTP) retorna `{ "error": "..." }` — a tela mostra a mensagem, igual
/// ao dashboard do Claude. A janela é `start`..`end` (range personalizado) quando
/// ambos vêm preenchidos; senão, `days` dias terminando hoje (data local). A API
/// cobre no máximo ~90 dias, então a janela é limitada a isso.
pub fn collect(
    client: &Client,
    auth_path: &str,
    days: u32,
    start: Option<String>,
    end: Option<String>,
) -> Value {
    let auth_path = auth_path.trim();
    if auth_path.is_empty() {
        return json!({ "error": "Caminho do auth.json do Codex nao configurado." });
    }

    let auth_raw = match fs::read_to_string(auth_path) {
        Ok(raw) => raw,
        Err(error) => {
            return json!({ "error": format!("Falha ao ler auth.json do Codex: {error}") })
        }
    };
    let auth: CodexAuth = match serde_json::from_str(&auth_raw) {
        Ok(auth) => auth,
        Err(error) => return json!({ "error": format!("auth.json invalido: {error}") }),
    };

    let tokens = auth.tokens;
    let token = tokens
        .as_ref()
        .and_then(|value| value.access_token.clone())
        .or_else(|| auth.openai.and_then(|value| value.access))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let token = match token {
        Some(token) => token,
        None => {
            return json!({
                "error": "Campos openai.access ou tokens.access_token nao foram encontrados no auth.json do Codex."
            })
        }
    };
    let account_id = tokens
        .and_then(|value| value.account_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    // Janela: range personalizado (start/end) ou preset (`days` dias até hoje).
    // Limita a 90 dias (máximo da API) e impede datas futuras.
    let today = Local::now().date_naive();
    let parse = |s: &str| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok();
    let (start, end) = match (start.as_deref().and_then(parse), end.as_deref().and_then(parse)) {
        (Some(mut s), Some(mut e)) => {
            if s > e {
                std::mem::swap(&mut s, &mut e);
            }
            if e > today {
                e = today;
            }
            let max_start = e - Duration::days(89);
            if s < max_start {
                s = max_start;
            }
            (s, e)
        }
        _ => {
            let days = days.clamp(1, 90);
            (today - Duration::days(i64::from(days - 1)), today)
        }
    };
    let url = format!(
        "https://chatgpt.com/backend-api/wham/usage/daily-token-usage-breakdown?start_date={}&end_date={}&group_by=day",
        start.format("%Y-%m-%d"),
        end.format("%Y-%m-%d"),
    );

    let mut request = client
        .get(&url)
        .header("accept", "*/*")
        .header("accept-language", "pt-BR,pt;q=0.9,en;q=0.8")
        .header("authorization", format!("Bearer {token}"))
        .header("cache-control", "no-cache")
        .header("pragma", "no-cache")
        .header("oai-language", "pt-BR")
        .header(
            "x-openai-target-path",
            "/backend-api/wham/usage/daily-token-usage-breakdown",
        )
        .header(
            "x-openai-target-route",
            "/backend-api/wham/usage/daily-token-usage-breakdown",
        );
    if let Some(account_id) = account_id {
        request = request.header("chatgpt-account-id", account_id);
    }

    let response = match request.send() {
        Ok(response) => response,
        Err(error) => return json!({ "error": format!("Falha HTTP ao consultar Codex: {error}") }),
    };
    if !response.status().is_success() {
        return json!({
            "error": format!("Codex retornou status HTTP {}.", response.status())
        });
    }

    let payload: BreakdownResponse = match response.json() {
        Ok(payload) => payload,
        Err(error) => {
            return json!({ "error": format!("Falha ao decodificar resposta do Codex: {error}") })
        }
    };

    // O `data` já vem no shape que o frontend consome (date,
    // product_surface_usage_values, models[]). Repassamos como está e anexamos
    // metadados.
    json!({
        "units": payload.units,
        "groupBy": payload.group_by,
        "days": payload.data,
        "generatedAt": Local::now().to_rfc3339(),
    })
}
