# Changelog

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
