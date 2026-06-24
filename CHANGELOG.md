# Changelog

Todas as alterações relevantes deste projeto são documentadas aqui.
O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

As releases são geradas automaticamente a cada push no `main`, com versão
`0.2.<run_number>` (o número da execução do CI). Por isso não há um cabeçalho
de versão fixo: mantenha as alterações da próxima release na seção **[Não
lançado]** logo abaixo. O workflow de release extrai o **corpo dessa primeira
seção** e o publica como as notas da versão (`notes` do `latest.json`), que
aparecem na janela de "Nova versão disponível" do app (OTA).

> Regra: preencha esta seção **antes** de fazer merge no `main`. Após a release,
> a seção pode ser limpa para acumular as próximas mudanças.

## [Não lançado]

### Adicionado
- O aviso de nova versão agora mostra as novidades da atualização em uma janela
  dedicada, com barra de progresso durante o download.

### Alterado
- As notas de cada versão passam a descrever as novidades de forma legível, em vez
  de um identificador técnico do build.

## Histórico

Versões anteriores à introdução deste arquivo não possuem changelog detalhado —
eram builds automáticas do `main` identificadas apenas pelo commit.
