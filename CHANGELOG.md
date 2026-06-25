# Changelog

Todas as alterações relevantes deste projeto são documentadas aqui.
O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

As releases são geradas automaticamente a cada push no `main`, com versão
`0.2.<run_number>` (o número da execução do CI). O próprio **app** lê este
`CHANGELOG.md` (do `main`) para exibir as novidades: a **janela de atualização**
(OTA) mostra o *delta* — as novidades de todas as versões entre a instalada e a
mais nova — e a tela **Novidades** mostra o histórico completo. O campo `notes` do
`latest.json` não é mais usado para isso (vai vazio).

> **Como manter (cumulativo):**
> - Acumule as mudanças da próxima versão em **[Não lançado]** — só o que é
>   visível ou perceptível pelo usuário, em linguagem padrão da indústria.
>   Preencha **antes** de fazer merge no `main`.
> - O histórico é **cumulativo**: nunca apague seções de versões já lançadas.
> - A cada novo ciclo, **antes** de registrar novas mudanças, promova a
>   `[Não lançado]` anterior para uma seção da versão que foi publicada
>   (`## [0.2.<run>] - AAAA-MM-DD`) e recrie uma `[Não lançado]` vazia no topo.
>   A versão publicada é o campo `version` do `latest.json` da release (ou
>   `0.2.<run_number>` da execução do workflow de release).

## [Não lançado]

### Adicionado
- Nova tela **Novidades**, com o histórico de versões do app.

### Alterado
- Ao atualizar pulando versões, o aviso de atualização agora mostra as novidades de
  **todas** as versões entre a sua e a mais nova, não só a da versão mais recente.

## [0.2.26] - 2026-06-24

### Adicionado
- O aviso de nova versão agora mostra as novidades da atualização em uma janela
  dedicada, com barra de progresso durante o download.

### Alterado
- As notas de cada versão passam a descrever as novidades de forma legível, em vez
  de um identificador técnico do build.

## Histórico

Versões anteriores à introdução deste arquivo (até a `0.2.25`) não possuem
changelog detalhado — eram builds automáticas do `main` identificadas apenas pelo
commit.
