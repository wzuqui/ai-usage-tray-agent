# Onboarding — ai-usage-tray-agent

Guia de **fluxo de trabalho** para quem for contribuir com o projeto. Cobre o que
**não dá pra inferir lendo o código**: topologia do git, como a release sai, por que
o CHANGELOG é crítico, os pontos sensíveis do OTA e as armadilhas técnicas já
aprendidas. Para a arquitetura/telas, veja o `README.md`.

---

## TL;DR (o mínimo pra não quebrar nada)

1. **Nunca commite direto no `main`.** Toda mudança vai por branch de tópico +
   Pull Request no upstream (`wzuqui/ai-usage-tray-agent`).
2. **A release sai SÓ do upstream**, automaticamente no merge para o `main` dele.
   Push no seu fork não publica nada.
3. **Sempre atualize o `CHANGELOG.md` antes de mergear** — o app lê esse arquivo em
   runtime pra mostrar as novidades/OTA. Sem isso, o usuário vê novidades vazias.
4. **A chave de assinatura do updater existe só no secret do upstream e não está no
   repo.** Se for perdida, nenhum app instalado aceita updates futuros.

---

## 1. Topologia do Git

- `origin` = fork pessoal (`GedsonAJr/ai-usage-tray-agent`) — só para desenvolvimento/PR.
- `upstream` = repositório oficial (`wzuqui/ai-usage-tray-agent`) — fonte de distribuição.
- O mantenedor atual é **owner do upstream** (pode configurar secrets/Settings do `wzuqui`).
- O upstream aceita PRs com **merge commit** (preserva os SHAs originais).
- O upstream tem um ruleset **"Code Review" sem bypass** → **tudo passa por PR**; o CI
  **não pode** commitar de volta no `main`.

## 2. Release — onde e como sai

- A release roda **apenas no upstream**. `.github/workflows/release.yml` tem um guard:
  `if: github.repository == 'wzuqui/ai-usage-tray-agent'`.
- **Push no `origin/main` (fork) NÃO dispara release.** Só o merge no `main` do
  upstream recria a release rolling `main-latest`.
- A versão é injetada como `0.2.<github.run_number>` a cada build e **precisa ser
  monotônica** (o updater compara semver).

## 3. Fluxo de contribuição (passo a passo)

Para cada mudança:

```sh
git fetch upstream
git switch -c <branch> upstream/main      # SEMPRE parta de upstream/main
# ... editar + commit ...
git push -u origin <branch>
gh pr create --repo wzuqui/ai-usage-tray-agent --base main --head GedsonAJr:<branch>
# após aprovar/mergear o PR, sincronize o fork (passo 4 abaixo)
```

Sincronizar o fork periodicamente (fast-forward simples):

```sh
git fetch upstream && git switch main && git merge upstream/main && git push origin main
```

> **Por que nunca commitar no `main` primeiro:** commitar no main e depois rebasear
> para o PR cria dois commits com o mesmo conteúdo (já aconteceu). Como o upstream usa
> merge commit, partir sempre de `upstream/main` mantém o histórico limpo.

## 4. CHANGELOG.md — é runtime, não cosmético

- **O app lê o `CHANGELOG.md` do `main` em tempo de execução** (comando `get_changelog`)
  para montar a janela de atualização OTA (mostra o *delta* entre a versão instalada e a
  mais nova) e a tela **Novidades** (histórico completo).
- O campo `notes` do `latest.json` **não é mais usado** (vai vazio).
- **Consequência:** sempre preencha o CHANGELOG **antes** de qualquer fluxo git que entre
  no `main`. Sem isso, o usuário vê novidades vazias/desatualizadas.

**Estilo:**
- **CHANGELOG.md** = resumido e voltado ao usuário final. Só o que é **visível/perceptível**,
  no padrão Keep a Changelog (Adicionado/Alterado/Corrigido/Removido/Obsoleto/Segurança).
  Sem nomes de arquivo, campos internos ou hashes.
- **Cada item em UMA ÚNICA LINHA** (sem quebra manual) — renderers de versões antigas
  quebram itens multi-linha de forma estranha na janela OTA.
- **Mensagem de commit** = completa e técnica (é nela que mora o changelog "detalhado").

**Modelo cumulativo (Keep a Changelog):**
- O arquivo mantém uma seção por versão lançada, que nunca é apagada.
- `[Não lançado]` no topo = a versão mais nova ainda não promovida; o app a mapeia para a
  versão alvo ao exibir.
- A cada novo ciclo, **antes** de registrar mudanças novas:
  1. Promova a `[Não lançado]` anterior para `## [0.2.<run>] - AAAA-MM-DD` (use o `version`
     do `latest.json` da release, ou `0.2.<run_number>` da run do Release —
     `gh run list --workflow=release.yml`). Se a `[Não lançado]` estava vazia, não crie seção.
  2. Recrie uma `[Não lançado]` vazia no topo e adicione ali as entradas novas.
- **Cuidado com o parser** (`parseChangelog`/`renderMarkdown` em `src/changelog.ts`): a guia
  "Como manter" no topo é blockquote (`>`), nunca `## `; subtítulos de categoria usam `### `
  (só `## ` vira seção).

## 5. OTA / Auto-update — pontos sensíveis

- Implementado com `tauri-plugin-updater` (v2), com o fluxo **no backend Rust**
  (`src-tauri/src/lib.rs`, `check_for_updates`), porque o app é tray-only e a webview nem
  sempre existe. Checa no boot (silencioso se não há update) e abre uma janela de novidades
  (`update.html`) mostrando o changelog antes de instalar. Há item "Buscar atualizações" no
  menu do tray. Cobre MSI (Windows) e AppImage (Linux).
- **Assinatura (CRÍTICO):** a chave privada (`tauri signer generate`, sem senha) está apenas
  no secret `TAURI_SIGNING_PRIVATE_KEY` do upstream. **Ela NÃO está no repositório. Se for
  perdida, nenhum update futuro é aceito pelos apps já instalados.**
- **Endpoint/pubkey:** `tauri.conf.json` → `bundle.createUpdaterArtifacts: true` +
  `plugins.updater` (pubkey embutida + endpoint
  `https://github.com/wzuqui/ai-usage-tray-agent/releases/latest/download/latest.json`).
  Ao mexer em release/versão/updater, mantenha versão monotônica e endpoint/pubkey coerentes.
- **Build local:** o build completo exige a chave de assinatura; use
  `npx tauri build --no-bundle` para pular o bundling/assinatura. Testes do dia a dia são em
  modo dev: `npm run tauri dev`.

## 6. Armadilhas técnicas já aprendidas

- **Comandos Tauri que criam/abrem uma `WebviewWindow` DEVEM ser `async fn`.** Um comando
  síncrono roda na thread principal e **trava o event loop** — a janela abre mas o webview
  nunca carrega (tela em branco). Isso causou um bug do OTA (janela de novidades em branco
  ao abrir pela tela "Sobre").
- **API de uso do Codex:** os dados de analytics vêm do backend `chatgpt.com/backend-api/wham/...`
  e abrem direto com o token do `~/.codex/auth.json`
  (`Authorization: Bearer <access_token>` + `chatgpt-account-id: <account_id>`). O namespace
  `wham/` funciona; o `codex/...` dá 403. O `access_token` expira (~10 dias) — **releia o
  `auth.json` a cada coleta** para pegar o token renovado. Endpoints principais:
  `wham/usage` (gauges) e `wham/usage/daily-token-usage-breakdown` (série temporal; unidade
  em **percentual**, não tokens).

## 7. Disciplina de documentação

Antes de **qualquer** commit, faça uma passada de consistência e inclua as mudanças de doc
**no mesmo PR** da feature:
- `README.md` reflete as telas/comandos/estrutura/fluxos atuais?
- `CHANGELOG.md` tem a entrada da versão? (ver seção 4)
- Alguma doc cita arquivo/função/fluxo que mudou?

---

*Este guia complementa o `README.md` (arquitetura e telas) e o `CHANGELOG.md` (histórico de
versões). Mantenha-o atualizado quando o fluxo de trabalho mudar.*
