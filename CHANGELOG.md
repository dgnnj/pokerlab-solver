# Changelog

## 0.3.3 — 2026-11-22
- Erros das mãos completas na revisão espaçada: uma decisão com nota Imprecisão ou Erro entra na fila de "Revisar erros". Se o ponto da árvore é um exercício do pacote, a revisão repete a mesma situação com a mesma mão; senão, traz outra situação do mesmo tema. Quando o pacote não tem exercícios daquele tipo de decisão (por exemplo, o BB liderando no flop), a decisão não entra na fila e a mão avisa. Cada decisão diz o que aconteceu, e o fim da mão mostra quantas foram para a revisão.
- Tema de cada decisão da mão (mesma classificação dos exercícios do pacote) gravado e exportado no CSV das mãos (coluna `theme`).
- Banco schema v4 (migração com backup automático): item de revisão vem de um exercício ou de uma decisão de mão.

## 0.3.2 — 2026-11-21
- Rever mãos antigas: botão "Rever" no histórico de mãos abre a mão como foi jogada (mesa, ação, resultado, nota e explicação de cada decisão), sem resolver o board de novo. Mãos interrompidas mostram o estado na última decisão.
- Banco schema v3 (migração com backup automático): instantâneo de cada mão em `hands.view_json`. Mãos jogadas até a v0.3.1 aparecem na revisão só com os dados gravados de cada decisão.

## 0.3.1 — 2026-11-21
- Exportar mãos completas (CSV): uma linha por decisão, com os dados da mão (cartas, board, linha, resultado) e a avaliação de cada decisão. Botão no histórico de mãos e em Configurações.
- Correção: o motivo de corte da nota na mão completa mostrava "frequência acumulada 0.000000%"; agora diz "abaixo de 0,001%" (o limite real).

## 0.3.0 — 2026-11-21
- Mão completa: jogar do flop ao river contra a estratégia do motor. O app resolve a árvore completa do flop escolhido em segundo plano (progresso, cancelamento, mesma configuração do pacote) e mantém um board na memória; mãos sorteadas juntas pelos ranges, adversário pela estratégia da mão dele, turn/river sem reposição, runout distribuído após all-in, showdown com ranking próprio de mãos.
- Nota por decisão (perda de EV), opção de ver as notas só no fim da mão, revisão das decisões, corte explicado quando a linha tem peso nulo na solução. Resultado da mão separado da perda de EV.
- Histórico de mãos (banco schema v2, migração com backup automático) com resultado, perda média por decisão e por posição.

## 0.2.0 — 2026-11-21
- Exercícios de turn e river: pacote `btn-vs-bb-srp-100bb` v1.1.0 exporta, para cada flop, as linhas check-check, c-bet pequena paga e c-bet grande paga com 5 cartas de turn representativas (traz 3ª do naipe, overcard, pareia a carta alta, baixa, do meio) e, nas duas primeiras, o river após check-check e aposta paga no turn com 3 cartas representativas. Ranges de cada ponto são os do motor para a linha jogada.
- Temas separados por rua e pelo que aconteceu na rua anterior ("após check-check", "após aposta paga").
- Treinar por rua (todos do flop/turn/river); Estudar com seletor de rua; Progresso filtra por categoria de carta do turn/river.
- Linha da mão exibida por rua, com a carta e a categoria de cada runout.

## 0.1.0 — 2026-11-20
- Prova de conceito do motor: `pokerlab-cli solve | verify-reload | query` com spot de river, formato de solução versionado (schema v1).
- Pacote inicial `btn-vs-bb-srp-100bb` v1.0.0: 12 flops, 47 exercícios, explorabilidade final 0,43–0,49% do pote.
- App desktop (Tauri 2): launcher com verificação de atualização assinada, treino de uma decisão por vez com feedback por EV, estudo (matriz 13x13 e combos), revisão de erros espaçada, progresso com filtros e tamanho de amostra, preset de estudo de ~30 min, exportação CSV e backup.
- Instalador NSIS por usuário; updater validado localmente com versões de teste.
