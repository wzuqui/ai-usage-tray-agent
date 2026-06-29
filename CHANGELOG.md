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
> - Escreva cada item em **uma única linha** (sem quebra manual): o aviso de
>   atualização é renderizado pelo app **já instalado** do usuário, e renderers
>   antigos podem exibir itens multi-linha de forma quebrada.

## [Não lançado]

## [0.2.37] - 2026-06-29

### Adicionado
- Configurações: nova aba **Servidor** para abrir os dashboards de uso no navegador, protegidos por um **PIN** (endereço, porta e PIN configuráveis; somente leitura — não expõe Configurações, Envio nem credenciais).

## [0.2.33] - 2026-06-29

### Adicionado
- Widget: novo seletor **Modo de exibição** com **Completo** (cards com barras, o atual), **Mínimo** (uma linha por provedor) e **Anel duplo** (anéis de progresso, sessão no anel externo e semanal no interno).
- Widget: opção **Nenhum** no formato do reset, que oculta o tempo/horário de reset em todos os modos.

### Alterado
- Widget: a janela pode ficar mais compacta (altura mínima menor), útil nos modos Mínimo e Anel duplo.
- Dashboard Claude: nas abas **Ferramentas** e **Projetos**, as barras de ranking ficaram com o mesmo comprimento e os nomes longos não são mais cortados.

### Corrigido
- Dashboard Codex: o indicador de carregamento não pisca mais ao reabrir a janela.

## [0.2.32] - 2026-06-25

### Corrigido
- O botão **Atualizar agora** na tela **Sobre** abria uma janela em branco; agora carrega as novidades normalmente.

## [0.2.31] - 2026-06-25

### Adicionado
- Selo **Atualização disponível** no item **Sobre** do menu quando há uma nova versão (verificada ao abrir o app).
- Dashboard Claude: novas visões **Ferramentas** (ferramentas mais usadas) e **Projetos** (uso por projeto).
- Dashboard Claude e Dashboard Codex: seletor de **intervalo de datas personalizado** (no Codex, até os últimos 90 dias).
- Dashboard Codex: indicador de carregamento e mensagem quando não há uso no período selecionado.

### Alterado
- Menu: **Uso atual** passou a ser o primeiro item, com **Envio de dados** logo abaixo.
- Dashboard Claude: abre nos **últimos 30 dias** por padrão e as cores do gráfico de modelos ganharam mais contraste.
- Telas mais limpas em **Uso atual**, **Dashboard Claude**, **Dashboard Codex** e **Configurações** (títulos e rodapés simplificados, sem subtítulos).

### Corrigido
- O ícone do Codex em **Uso atual** deixava de exibir o fundo ao trocar de tela.

## [0.2.30] - 2026-06-25

### Corrigido
- O changelog no aviso de atualização não quebra mais as linhas no meio das frases.

## [0.2.29] - 2026-06-25

### Adicionado
- Nova tela **Sobre** no menu: versão instalada, verificação de atualização (com **Atualizar agora** quando houver uma nova versão) e as **Novidades**.

### Alterado
- As **Novidades** deixaram de ser um item próprio do menu e agora ficam dentro da tela **Sobre** (em uma área de altura fixa com rolagem).

## [0.2.28] - 2026-06-25

### Adicionado
- Nova tela **Novidades**, com o histórico de versões do app.

### Alterado
- Ao atualizar pulando versões, o aviso de atualização agora mostra as novidades de **todas** as versões entre a sua e a mais nova, não só a da versão mais recente.

## [0.2.26] - 2026-06-24

### Adicionado
- O aviso de nova versão agora mostra as novidades da atualização em uma janela dedicada, com barra de progresso durante o download.

### Alterado
- As notas de cada versão passam a descrever as novidades de forma legível, em vez de um identificador técnico do build.

## Histórico

Versões anteriores à introdução deste arquivo (até a `0.2.25`) não possuem
changelog detalhado — eram builds automáticas do `main` identificadas apenas pelo
commit.
