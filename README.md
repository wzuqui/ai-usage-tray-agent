# ai-usage-tray-agent

Aplicativo desktop cross-platform para Windows e Linux que roda no tray, coleta uso real de Codex e Claude e envia métricas para Grafana Loki.

O projeto foi feito com:

- Tauri v2
- Rust no backend
- Vite + TypeScript no frontend (UI nativa em webview)

## O que ele faz

- Inicia no tray sem abrir janela principal para o usuário
- Tem uma janela nativa (webview) com **Dashboard** e **Configurações**
- Coleta uso do Codex e do Claude em intervalo configurável
- Envia logs estruturados JSON para Loki
- Mantém logs locais
- Mostra status resumido no tray
- No Windows, exibe o uso direto na barra de tarefas
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

As edições no arquivo são aplicadas **em ~1s, sem reiniciar**: o app detecta a
mudança pelo `mtime` durante a espera entre coletas e reaplica a configuração na
hora (posição/fonte/cor da barra, lado, visibilidade dos provedores e o próprio
`intervaloSegundos`). Isso **não** dispara um envio extra ao Loki — o intervalo
de envio é preservado.

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
      "mostraNaTaskbarWindows": true,
      "authJsonPath": "C:\\Users\\usuario\\.codex\\auth.json"
    },
    "claude": {
      "habilitado": true,
      "mostraNaTaskbarWindows": true,
      "organizationId": "org_exemplo",
      "cookie": "sessionKey=..."
    }
  },
  "barraTarefas": {
    "lado": "direita",
    "deslocamento": 0,
    "tamanhoFonte": 9,
    "corFonte": "auto"
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

## Tray (ícone na bandeja)

- **Clique esquerdo** no ícone: abre a janela do app (Dashboard/Configurações).
- **Clique direito** no ícone: abre o menu.

Itens do menu:

- Status atual
- Codex atual
- Claude atual
- **Abrir**: abre a janela do app (mesma ação do clique esquerdo).
- Abrir `config.json`
- Abrir pasta de logs
- Enviar agora
- Pausar/retomar coleta
- Sair

> A exibição de cada IA na barra de tarefas e a inicialização automática não são
> itens do tray; são editadas nas **Configurações** do app (abas Barra de
> tarefas e Sistema).

## Janela do app (Dashboard e Configurações)

A interface é uma janela nativa (webview do Tauri), aberta pelo clique esquerdo
no tray ou pelo item **Abrir**. Não usa navegador nem servidor HTTP local: o
frontend conversa com o backend Rust por comandos (IPC). Um menu lateral troca
entre duas seções. Fechar pela janela (X) **esconde** o app (continua no tray).

### Dashboard

Replica o painel de uso do Claude Code lendo as mesmas fontes locais
(`~/.claude/projects/**/*.jsonl` e `~/.claude/stats-cache.json`): cards de
resumo, heatmap de atividade e gráfico de tokens por modelo. Os dados vêm do
comando `get_stats` e são recarregados ao reabrir a janela.

### Configurações

Formulário com **abas** que cobre **todas as opções do `config.json`** (mais o
"iniciar com o sistema"):

- **Geral**: `usuario`, `intervaloSegundos`, `loki.url`.
- **Codex**: `habilitado`, `authJsonPath`, `mostraNaTaskbarWindows`.
- **Claude**: `habilitado`, `organizationId`, `cookie` (com mostrar/ocultar),
  `mostraNaTaskbarWindows`.
- **Barra de tarefas** (Windows): `lado`, `deslocamento`, `tamanhoFonte`,
  `corFonte` (com seletor de cor).
- **Sistema**: **Iniciar com o sistema** (autostart) — não fica no `config.json`,
  é gerenciado pelo `tauri-plugin-autostart`.

Ao salvar, o `config.json` é gravado (com normalização: clamp de intervalo/fonte,
validação de cor) e o app aplica tudo em ~1s, **sem reiniciar e sem disparar um
envio extra** ao Loki. O autostart é aplicado na hora.

## Inicialização automática

O app usa o `tauri-plugin-autostart` (chave `HKCU\...\Run` no Windows) e vem
**habilitado por padrão na primeira execução**. Depois disso:

- O estado é controlado pela opção **Iniciar com o sistema** na aba **Sistema**
  das Configurações.
- Se continuar ligado, o caminho do executável é reaplicado a cada início
  (evita apontar para um caminho antigo após atualizar/reinstalar).
- Se o usuário desligar pelas Configurações, permanece desligado nas próximas
  execuções.

## Barra de tarefas (Windows)

No Windows o app desenha o uso diretamente na barra de tarefas. Cada provedor
**habilitado** vira um elemento separado, com duas linhas: o nome e, abaixo,
`uso da sessão (5h)` e `uso semanal (7d)`, cada um com o tempo até o reset:

```text
        Claude
61% (3:33h) | 40% (5d)
```

- O primeiro valor é o uso da janela de 5h e quanto falta para resetar.
- O segundo valor é o uso dos últimos 7 dias e quanto falta para resetar.
- Provedores com `"habilitado": false` no `config.json` não aparecem na barra.
- A exibição de cada provedor na barra é controlada por
  `providers.<ia>.mostraNaTaskbarWindows` (padrão `true`). Você pode alterar isso
  pelas **Configurações** do app ou editando o `config.json` direto; nos dois
  casos vale em ~1s. Só aparece na barra quando `habilitado` **e**
  `mostraNaTaskbarWindows` forem `true`.
- Em Linux/macOS o campo `mostraNaTaskbarWindows` é lido mas **ignorado**: o
  widget da barra de tarefas só existe no Windows. O campo é mantido no arquivo
  para que a mesma `config.json` seja portável entre sistemas.

Como funciona:

- A Microsoft removeu o suporte a *deskbands* na barra de tarefas reescrita do
  Windows 11, então o texto não é uma deskband COM clássica.
- Em vez disso, o app cria uma pequena janela por provedor e a torna *filha* da
  janela da barra (`Shell_TrayWnd`) via Win32 `SetParent`. O texto fica de fato
  dentro da barra.
- A cor do texto é escolhida automaticamente pela cor real da barra
  (tema claro/escuro e *accent color*), para manter contraste.
- A janela é reposicionada periodicamente e recriada sozinha se o Explorer
  reiniciar.

Posicionamento:

- O lado é controlado por `config.json` → `barraTarefas.lado`: `"direita"`
  (padrão) ou `"esquerda"`. O cálculo que "adivinha" a posição é espelhado
  conforme o lado.
- **Direita** (padrão): o widget fica à esquerda da área de notificação
  (bandeja). Se houver outros widgets de terceiros embutidos na faixa direita da
  barra (monitores de rede, etc.), ele detecta e se ancora à esquerda deles,
  para conviverem sem sobreposição.
- **Esquerda**: o widget fica na ponta esquerda da barra, ancorado à direita de
  eventuais widgets ali (ex.: botão de Widgets/clima). É útil com o menu Iniciar
  **centralizado**, que deixa a ponta esquerda livre. Não é recomendado com a
  barra alinhada à esquerda (Iniciar/apps na esquerda), pois o espaço já está
  ocupado — nesse caso use o lado direito.
- Ajuste fino manual: `config.json` → `barraTarefas.deslocamento` (px). Negativo
  move o widget para a esquerda, positivo para a direita, **em ambos os lados**.
  Útil quando há *toolbars*/atalhos de pasta na barra (Windows 10) que não são
  detectados automaticamente — ajuste até liberar o espaço.

Aparência (fonte):

- `barraTarefas.tamanhoFonte` (pontos, padrão `9`, limitado a 6–24): tamanho da
  fonte do texto na barra.
- `barraTarefas.corFonte`: `"auto"` (padrão — preto em barra clara, branco em
  barra escura, conforme a cor real da barra) ou um hex `"#RRGGBB"` (ex.:
  `"#FFD700"`). Valores inválidos voltam para `"auto"`.
- Ambos são aplicados em ~1s ao editar o `config.json`, sem reiniciar. Evite uma
  `corFonte` igual à cor da barra (o fundo é transparente por *color-key*, então
  o texto sumiria).

Limitações:

- Funciona na barra padrão do Windows 11; barras modificadas
  (ExplorerPatcher/StartAllBack) podem se comportar de forma diferente.
- A mistura com barras translúcidas/acrílicas é aproximada (color-key), não um
  blend perfeito.

No Linux esse recurso não se aplica; o uso continua disponível no tooltip e no
título do tray.

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

- Windows: `.msi` (instalador) e `AiUsageTrayAgent-portable.exe` (portátil, sem instalar)
- Linux: `AppImage`

Arquivos relevantes:

- Workflow único de build/release: [.github/workflows/release.yml](.github/workflows/release.yml)
- Config Windows: [src-tauri/tauri.windows.conf.json](src-tauri/tauri.windows.conf.json)
- Config Linux: [src-tauri/tauri.linux.conf.json](src-tauri/tauri.linux.conf.json)

O workflow de release roda:

- automaticamente em `push` para `main`
- e sempre recria a release `main-latest` com os artefatos mais recentes

Para publicar no GitHub Releases, garanta que o repositório permita `GITHUB_TOKEN` com permissão de escrita em Actions.

## Dashboard Grafana

Exemplo sanitizado:

- [docs/grafana-dashboard.example.json](docs/grafana-dashboard.example.json)

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
index.html            # janela unica do app (menu lateral + secoes)
src/
  main.ts             # shell: navegacao entre Dashboard e Configuracoes
  dashboard.ts        # dashboard de uso (consome o comando get_stats)
  settings.ts         # configuracoes (consome get_settings/save_settings)
  styles.css

src-tauri/
  src/
    lib.rs             # tray, worker de coleta e comandos IPC (get_stats/get_settings/save_settings)
    main.rs
    usage_dashboard.rs # coleta as estatisticas do dashboard
    taskbar_widget.rs  # widget da barra de tarefas (somente Windows)
  tauri.conf.json
  tauri.windows.conf.json
  tauri.linux.conf.json

docs/
  teste.http
  grafana-dashboard.example.json
```

## Próximos passos

- renovação e tratamento melhor de credenciais expiradas
- mais logs de sucesso no backend
- pacote Linux adicional como AppImage, se fizer sentido
