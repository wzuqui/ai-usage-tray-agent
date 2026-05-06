# ai-usage-tray-agent

Aplicativo desktop cross-platform para Windows e Linux que roda no tray, coleta uso real de Codex e Claude e envia métricas para Grafana Loki.

O projeto foi feito com:

- Tauri v2
- Rust no backend
- Vite + TypeScript no frontend mínimo

## O que ele faz

- Inicia no tray sem abrir janela principal para o usuário
- Coleta uso do Codex e do Claude em intervalo configurável
- Envia logs estruturados JSON para Loki
- Mantém logs locais
- Mostra status resumido no tray
- Permite pausar, retomar e forçar envio pelo menu do tray

## Status atual

MVP funcional com:

- Coleta real do Codex usando `auth.json`
- Coleta real do Claude usando `organizationId` e `sessionKey`
- Envio para Loki sem `tenant` e sem `basic auth`
- Empacotamento planejado para:
- Windows: instalador `.msi`
- Linux: `AppImage`

## Configuração

O app cria e lê `config.json` automaticamente.

Windows:

- Config: `%AppData%/AiUsageTrayAgent/config.json`
- Logs: `%LocalAppData%/AiUsageTrayAgent/logs/`

Linux:

- Config: `~/.config/ai-usage-tray-agent/config.json`
- Logs: `~/.local/state/ai-usage-tray-agent/logs/`

Exemplo:

```json
{
  "usuario": "usuario-exemplo",
  "intervaloSegundos": 10,
  "loki": {
    "url": "http://loki.exemplo.local:3100/loki/api/v1/push"
  },
  "providers": {
    "codex": {
      "habilitado": true,
      "authJsonPath": "C:\\Users\\usuario\\.codex\\auth.json"
    },
    "claude": {
      "habilitado": true,
      "organizationId": "org_exemplo",
      "cookie": "sessionKey=..."
    }
  }
}
```

## Formato enviado para o Loki

Labels:

- `app = "ai-usage-tray-agent"`
- `usuario`
- `ferramenta`
- `host`

Payload interno:

```json
{
  "uso_percentual": 47.0,
  "restante_percentual": 53.0,
  "status": "ok",
  "reset_em": "2026-05-06T17:00:41+00:00"
}
```

O timestamp do Loki é enviado em nanossegundos no campo `values`.

## Menu do tray

- Status atual
- Codex atual
- Claude atual
- Abrir `config.json`
- Abrir pasta de logs
- Enviar agora
- Pausar/retomar coleta
- Sair

## Rodando localmente

Pré-requisitos:

- Node.js
- Rust
- Dependências do Tauri v2 para sua plataforma

Comandos:

```bash
npm install
npm run tauri dev
```

Build local:

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

## Instaladores

O repositório já está preparado para gerar:

- Windows: `.msi`
- Linux: `AppImage`

Arquivos relevantes:

- Workflow único de build/release: [.github/workflows/release.yml](C:/Projetos/codex/ai-usage-tray-agent/.github/workflows/release.yml)
- Config Windows: [src-tauri/tauri.windows.conf.json](C:/Projetos/codex/ai-usage-tray-agent/src-tauri/tauri.windows.conf.json)
- Config Linux: [src-tauri/tauri.linux.conf.json](C:/Projetos/codex/ai-usage-tray-agent/src-tauri/tauri.linux.conf.json)

O workflow de release roda:

- automaticamente em `push` para `main`
- quando o commit indicar uma nova versão, como `feat:` ou `fix:`

Para publicar no GitHub Releases, garanta que o repositório permita `GITHUB_TOKEN` com permissão de escrita em Actions.

## Dashboard Grafana

Exemplo sanitizado:

- [docs/grafana-dashboard.example.json](C:/Projetos/codex/ai-usage-tray-agent/docs/grafana-dashboard.example.json)

Esse arquivo mantém:

- gráfico por usuário/ferramenta
- gauges de Codex e Claude
- tabela com últimos logs
- variáveis de filtro por `usuario` e `ferramenta`

## Limitações conhecidas

Linux:

- O suporte a tray depende do ambiente gráfico
- GNOME pode exigir suporte adicional a AppIndicator/StatusNotifierItem
- Tooltip de tray pode variar entre distribuições

Claude:

- A coleta depende de um `sessionKey` válido
- Se o cookie expirar, será necessário atualizar o `config.json`

Codex:

- A coleta depende de um `auth.json` válido
- O formato atual suportado inclui `tokens.access_token`

## Estrutura do projeto

```text
src/
  main.ts
  styles.css

src-tauri/
  src/
    lib.rs
    main.rs
  tauri.conf.json
  tauri.windows.conf.json
  tauri.linux.conf.json

docs/
  teste.http
  grafana-dashboard.example.json
```

## Próximos passos

- opção de iniciar com o sistema operacional
- renovação e tratamento melhor de credenciais expiradas
- mais logs de sucesso no backend
- pacote Linux adicional como AppImage, se fizer sentido
