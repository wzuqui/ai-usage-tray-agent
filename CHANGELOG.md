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

### Adicionado
- Configurações → Barra de tarefas e Widget: aviso, em cada provedor, quando ele está desativado ou sem credenciais (o interruptor continua operável).

### Alterado
- Configurações → Servidor: o liga/desliga ganhou destaque — virou um card com interruptor no cabeçalho, e os campos (endereço, porta, PIN) ficam esmaecidos quando o servidor está desligado.

## [0.2.43] - 2026-07-01

### Adicionado
- Configurações → Barra de tarefas e Widget: miniatura ilustrativa de como o widget aparece e onde fica na tela.
- Configurações → Widget: seletor do modo de exibição (Completo, Mínimo e Anel duplo) com uma prévia de cada modo — clicar na miniatura escolhe o modo.

### Alterado
- Configurações: nova aba **Envio** reúne o nome de exibição, a URL do Loki e o "Enviar ao Loki" de cada provedor (agora como interruptores, com aviso quando o provedor está desativado ou sem credenciais).
- Configurações → Codex e Claude: cada provedor virou um card com o logo e um interruptor para ativar/desativar a coleta; os campos de credenciais ficam esmaecidos quando o provedor está desativado.
- Configurações → Geral: a opção "Iniciar com o sistema" passou para esta aba (rótulo unificado em todos os sistemas operacionais).
- Configurações → Widget: o widget passa a aparecer quando ao menos um provedor está marcado (o interruptor separado "Mostrar widget na área de trabalho" foi removido).
- Configurações: os campos obrigatórios de cada provedor e o PIN do servidor agora são sinalizados como obrigatórios, com um aviso quando faltam para a coleta/servidor funcionar.

### Removido
- Configurações: aba **Sistema** — a opção "Iniciar com o sistema" foi para a aba Geral.

## [0.2.42] - 2026-06-30

### Alterado
- Envio de dados: a **contagem regressiva** do próximo envio passou para o **subtítulo da página** (como o "Atualizado há…" do Uso atual), e o histórico de envios ganhou um divisor abaixo do título.
- Janela de atualização: visual mais limpo — sem a seta ao lado do título e sem o sufixo "Atualização disponível" na barra de título da janela.
- Dashboard Claude e Dashboard Codex: divisor entre as abas/seletor de período e o conteúdo.

### Corrigido
- Abas: a aba ativa não perde mais o destaque ao alternar entre telas (Dashboard Claude, Dashboard Codex e Configurações).

### Removido
- Menu do tray (clique direito): removidos os itens de **status** (status geral e uso por provedor Codex/Claude) e o botão **Enviar agora** — o menu ficou só com as ações.

## [0.2.41] - 2026-06-30

### Corrigido
- Dashboard Claude: o painel não fica mais travado até reiniciar o app caso ocorra um erro ao carregar os dados de uso.

## [0.2.40] - 2026-06-30

### Adicionado
- Envio de dados: aviso quando **nenhum provedor está com "Enviar ao Loki" ativado** — o indicador muda para "Nenhum provedor enviando" e o histórico mostra um lembrete para ativar Claude ou Codex em Configurações.
- Envio de dados: o **histórico de envios** agora mostra os **dados enviados** ao Loki em cada envio com sucesso (uso da sessão 5h, uso semanal 7d e o reset), com o payload completo no tooltip.

### Alterado
- Uso atual: o indicador **"Atualizado há…"** saiu dos cards para o **subtítulo da página** (aparece uma única vez).

### Removido
- Uso atual: botão **Atualizar agora** (a tela já atualiza sozinha a cada poucos segundos).

### Corrigido
- Envio de dados: o histórico não pisca mais todas as linhas ao voltar de outra aba — só novas entradas que chegam com a tela aberta são destacadas.

## [0.2.39] - 2026-06-29

### Adicionado
- Configurações: opção **Enviar ao Loki** em cada provedor (Codex e Claude).
- Envio de dados: indicador "ao vivo" com **contagem regressiva** do próximo envio e o **status de envio de cada provedor**; o histórico **destaca** as novas entradas.

### Alterado
- Tela **Sobre**: primeira seção reorganizada (versão e status de atualização na mesma linha; repositório logo abaixo) e a lista de **Novidades** sem a caixa interna.
- Tela **Envio de dados**: visual simplificado.
- Widget: nome do provedor em cinza e o tempo de reset em branco, em todos os modos de exibição.

### Removido
- Envio de dados: a seção **Envio por provedor** (agora nas Configurações de cada provedor), o botão **Enviar agora** e a informação "Último envio com sucesso".

### Corrigido
- Tela Sobre: o **Copiar link** (menu do botão direito) do repositório agora copia o endereço correto.

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
