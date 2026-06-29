# CLAUDE.md

Orientações para o Claude Code (e qualquer agente) ao trabalhar neste repositório.

## ⚠️ Leitura obrigatória antes de qualquer mudança

**Leia o [`ONBOARDING.md`](./ONBOARDING.md) primeiro.** Ele cobre o que **não dá
para inferir lendo o código**: topologia do git, como a release sai, por que o
`CHANGELOG.md` é crítico (é lido em runtime pelo app), os pontos sensíveis do OTA
e as armadilhas técnicas já aprendidas. Para arquitetura e telas, veja o
[`README.md`](./README.md).

## Regras que não podem ser quebradas (resumo do ONBOARDING)

1. **Nunca commite direto no `main`.** Toda mudança vai por **branch de tópico +
   Pull Request**. Crie a branch sempre a partir de `origin/main` atualizado.
2. **Atualize o `CHANGELOG.md` antes de mergear** — o app lê esse arquivo em
   runtime para as Novidades/OTA. Cada item em **uma única linha**, voltado ao
   usuário final (padrão Keep a Changelog), na seção `[Não lançado]`.
3. **A release sai só do upstream**, no merge para o `main` dele. Não tente
   publicar de outra forma; não mexa na chave de assinatura do updater.
4. **Doc no mesmo PR da feature**: confira se `README.md` (telas/comandos/estrutura)
   e `CHANGELOG.md` refletem a mudança.

## Armadilhas técnicas recorrentes

- **Comandos Tauri que abrem/criam uma `WebviewWindow` DEVEM ser `async fn`** — um
  comando síncrono trava o event loop e a janela abre em branco.
- **API de uso do Codex**: dados vêm de `chatgpt.com/backend-api/wham/...` com o
  token do `~/.codex/auth.json`; releia o `auth.json` a cada coleta (o token
  expira). O namespace `wham/` funciona; `codex/...` dá 403.

## Verificação local antes de commitar

```sh
npm run build                                  # tsc + vite (type-check + bundle)
cargo check --manifest-path src-tauri/Cargo.toml   # backend Rust
```

Para o dia a dia da UI, rode em modo dev com `npm run tauri dev`.
