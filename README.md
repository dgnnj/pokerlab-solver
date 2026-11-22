# PokerLab — treinador desktop de poker cash game (NLHE)

Aplicativo Windows 11 x64, em português, para treinar decisões pós-flop offline com estratégias e EVs calculados pelo [`postflop-solver`](https://github.com/b-inary/postflop-solver). Histórico local em SQLite, launcher com atualização assinada via GitHub Releases.

Especificação: `Especificacao_Treinador_Poker_Cash_Game.md` (v1.0). Decisões: [docs/adr/0001-fundacao.md](docs/adr/0001-fundacao.md).

## Estado (2026-11-22)

| Item | Estado |
|---|---|
| PoC do motor (river): solve, export versionado, recarga em outro processo, consulta | **Executado** |
| Testes de mapeamento combo/ação/frequência/EV, cartas bloqueadas, rake, perda de EV | **Executado** (57 testes Rust, 6 Vitest) |
| Pacotes de spots, 12 flops cada, flop + turn + river em linhas principais com cartas representativas: BTN x BB pote simples v1.2.0 (BB pode liderar no flop), CO x BB pote simples v1.0.0, SB x BB pote simples v1.0.0, BB x BTN pote 3-bet v1.0.0 | **Gerados e verificados** (explorabilidade < 0,5% do pote em todos os flops); ainda não publicados |
| App: launcher, treino, feedback por EV, estudo 13x13, revisão de erros, progresso, preset ~30 min, CSV, backup | **Executado** no app instalado (E2E via WebView2/CDP) |
| Persistência após fechar e reabrir | **Executado** (E2E no app instalado) |
| Instalador NSIS por usuário | **Executado** nesta máquina (Windows 11 Pro) |
| Updater 0.1.0 → 0.1.1 com feed local e chave dev, incl. falhas | **Executado** (11/11 checks) |
| Updater real pelo feed público (chave de produção), com adiar e histórico preservado | **Executado** nesta máquina: 0.3.0 → 0.3.1, 0.3.1 → 0.3.2 e 0.3.2 → 0.3.3 (8/8 passos cada; o banco migrou do schema v2 para o v3 na 0.3.2 e do v3 para o v4 na 0.3.3, com backup antes; `%LOCALAPPDATA%` virtualizado, não substitui a VM limpa) |
| CI (`ci.yml`) e release (`release.yml`) no GitHub | **Executado** (releases v0.1.0 a v0.3.3 publicadas) |
| Teste em VM Windows 11 limpa, SmartScreen, Authenticode | **Pendente** |
| Publicação com chave de produção e feed `dgnnj/pokerlab-solver` | **Executado** (v0.3.3 é a Latest; `latest.json` assinado e verificado contra a chave pública de produção) |
| Simulação de mão completa (flop ao river contra a política do motor), árvore do flop resolvida no app | **Executado** no build release (E2E via WebView2: 9h6h3h como BB e 8s7s5d como BTN, 3 mãos cada, histórico após reabrir); publicada na v0.3.0; exportação das mãos em CSV publicada na v0.3.1; revisão de mãos antigas publicada na v0.3.2; erros das mãos na revisão espaçada publicados na v0.3.3; com os 4 spots, E2E no build release em 9h6h3h como BB no pote 3-bet, 8s7s5d como SB no SB x BB e 9h6h3h como BB no BTN x BB 1.2.0 (E2E rodou no build release, não no instalador baixado do feed) |

## Pré-requisitos

- Rust stable (MSVC) e Visual Studio 2022 Build Tools com workload C++.
- Node.js 24 + npm.
- WebView2 Runtime (já presente no Windows 11).

Como o projeto está no OneDrive, mantenha os artefatos de build fora dele:

```powershell
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\pokerlab-target"
```

## Reproduzir

```bash
npm ci
cargo test --workspace
npm run typecheck
npm test
```

PoC do motor — resolver, salvar, recarregar em processo separado e consultar:

```bash
cargo run --release -p pokerlab-cli -- solve data/spots/river_poc.toml --out out/river_poc.solution.json --probe out/river_poc.probe.json --probe-combos AhKd,9h9d,6s5s
```

```bash
cargo run --release -p pokerlab-cli -- verify-reload out/river_poc.solution.json out/river_poc.probe.json
```

```bash
cargo run --release -p pokerlab-cli -- query out/river_poc.solution.json --node x-b1000 --combo AhKd --action 0
```

Pacotes de spots (um por arquivo em `data/packs/`, cada um com seu template em `data/spots/`; reaproveita cache quando spot, motor e plano de exportação não mudaram). O plano `[export]` de cada pacote escolhe as linhas de flop/turn e quantas cartas representativas de turn e river exportar. No Ryzen 5 7600X: BB x BTN pote 3-bet ≈4 min (até 1 GB de RAM por flop), CO x BB ≈20 min (até 4,5 GB), SB x BB ≈26 min (até 6,9 GB), BTN x BB ≈28 min (até 6,2 GB):

```bash
cargo run --release -p pokerlab-cli -- gen-pack data/packs/btn_vs_bb_srp_100bb.toml --out src-tauri/resources/packs/btn-vs-bb-srp-100bb
```

```bash
for p in src-tauri/resources/packs/*/; do cargo run --release -p pokerlab-cli -- verify-pack "$p"; done
```

App em desenvolvimento e instalador:

```bash
npx tauri dev
```

```bash
npx tauri build
```

O build com `createUpdaterArtifacts` exige `TAURI_SIGNING_PRIVATE_KEY` (use a chave **dev** gerada por `npx tauri signer generate -w .keys/updater-dev.key`, que fica fora do git) ou desative: `npx tauri build --config src-tauri/tauri.ci.conf.json`.

E2E da mão completa (resolve um flop no app, joga mãos até o fim, fecha, reabre e confere o histórico). Use o build **release**: no build debug o solve fica ~20x mais lento.

```bash
node tests/e2e/hand-sim.mjs --exe "$LOCALAPPDATA/pokerlab-target/release/pokerlab.exe" --pack src-tauri/resources/packs --spot "BB x BTN" --board 9h6h3h --hands 3 --hero BB
```

E2E da interface no app instalado (treina, fecha, reabre, confere histórico):

```bash
node tests/e2e/ui-persistence.mjs --exe "$LOCALAPPDATA/PokerLab/pokerlab.exe" --data out/e2e-data --shots out/e2e-shots
```

Ensaio do updater (gera duas versões de teste com identificador próprio e feed em `127.0.0.1:8765`):

```bash
npx tauri build -c src-tauri/tauri.updater-test.conf.json
```

Copie o setup 0.1.0 para `out/updater-test/v0.1.0/`, gere a 0.1.1 com `-c` adicional `{"version":"0.1.1"}` em `out/updater-test/v0.1.1/` e rode:

```bash
node tests/e2e/updater.mjs
```

Atualização real pelo feed público, depois de publicar uma release (instala a anterior em `%LOCALAPPDATA%\PokerLab`; dados em `--data`, por padrão na pasta temporária do sistema, fora do OneDrive; aborta após `--timeout` segundos, padrão 600):

```bash
gh release download v0.3.2 -p 'PokerLab_0.3.2_x64-setup.exe' -D out/updater-live
node tests/e2e/updater-live.mjs --setup out/updater-live/PokerLab_0.3.2_x64-setup.exe --from 0.3.2 --to 0.3.3
```

Com `--data` dentro de uma pasta sincronizada pelo OneDrive o teste já travou sem saída e, em outra execução, o app não criou o banco; use uma pasta local.

## Estrutura

```text
crates/poker-core      domínio: cartas, spots, formato de solução, pacotes, avaliação de EV, ranking de mãos (sem o motor)
crates/solver-adapter  único módulo que usa o postflop-solver (build, solve, navegação, export, simulação de mão)
crates/pokerlab-cli    solve / query / verify-reload / gen-pack / verify-pack
data/spots             spots versionados (TOML): PoCs de turn e river e um template por pacote (ranges, tamanhos, contabilidade pré-flop)
data/packs             configuração de cada pacote (lista de boards, plano de exportação)
src-tauri              app Tauri: SQLite, treinador, updater, pacotes incluídos em resources/packs/
src                    interface React
tests/e2e              testes ponta a ponta do app instalado e do updater
.github/workflows      CI e release em rascunho
```

## Mão completa

O pacote só guarda as linhas principais, então a mão inteira usa a árvore completa do flop, resolvida no próprio app em segundo plano com o spot gravado na solução do pacote (mesmo hash de configuração; o app mostra se difere). No Ryzen 5 7600X, por board: pote 3-bet 5–25 s e até 1 GB; CO x BB, SB x BB e BTN x BB de 30 s a 3–4 min e 1,3–6,9 GB (acima de 6 GiB estimados o motor usa armazenamento comprimido, com cerca de metade da memória). Desde o BTN x BB 1.2.0 o BB pode liderar no flop, o que deixou a árvore desse spot maior (antes 25–60 s e até 4,2 GB). Uma árvore por vez fica na memória enquanto o app está aberto; as mãos seguintes no mesmo board são instantâneas.

- Mãos privadas sorteadas juntas pelos pesos dos ranges, sem cartas em comum; o adversário sorteia cada ação pela estratégia da mão dele no nó; turn e river saem do baralho restante. All-in pago antes do river é nó terminal no motor (ele faz a média dos runouts), então o app distribui as cartas restantes para o showdown.
- Cada decisão é avaliada pela perda de EV no mesmo nó e mão. Se a sua linha leva a mão a peso praticamente nulo na solução (< 0,001% do peso inicial), a nota termina na última decisão avaliável e o motivo aparece.
- Resultado no pós-flop (o pote inicial conta para quem leva) separado da qualidade das decisões. Histórico em `hands` / `hand_decisions` (criadas no schema v2; migrações com backup).
- Rever mãos antigas: "Rever" no histórico abre a mão como foi jogada (mesa, cartas dos dois, ação, resultado e a nota de cada decisão com tabela de ações e explicação), lida do banco, sem precisar resolver o board de novo. O app grava um instantâneo da mão a cada decisão (`hands.view_json`, schema v3). Mãos jogadas até a v0.3.1 não têm instantâneo e mostram só os dados gravados de cada decisão.
- Erros da mão na revisão espaçada: decisão com nota Imprecisão ou Erro entra em "Revisar erros" (tabela `reviews` com `hand_decision_id`, schema v4). Se o ponto da árvore é exercício do pacote (mesmo `solution_id:node_id`), a revisão repete a situação com a mesma mão; senão, usa outro exercício do mesmo tema (classificação em `poker_core::theme`, a mesma do gerador de pacotes). Sem exercício do tema no pacote (ex.: BB na raiz do flop), não entra na fila e a decisão avisa. Publicado na v0.3.3.
- Exportar mãos completas (CSV), no histórico de mãos ou em Configurações: grava `exports\pokerlab-maos-<timestamp>.csv` com uma linha por decisão do aluno e os dados da mão repetidos em cada linha (cartas dos dois, board final, linha, vencedor, resultado, nota, tema, EVs, frequência, equidade, motivo de corte). Mão interrompida antes da primeira decisão sai com uma linha e as colunas da decisão vazias; `ended_at` vazio indica mão interrompida.

## Spots

Cada pacote é um spot (pré-flop fixo, 100 bb, sem rake) com 12 flops. O app instala todos os pacotes incluídos e cada tela de treino, estudo, mão completa e progresso tem um seletor "Spot". Ranges são premissas de estudo próprias (em `data/spots/`), não soluções pré-flop.

| Spot | Pré-flop | Pote / stack no flop | Ranges (% das mãos) |
|---|---|---|---|
| BTN x BB, pote simples (v1.2.0) | BTN abre 2,5 bb, BB paga | 5,5 bb / 97,5 bb | BTN 38,5%; BB paga 27,4% (37,4% com o 3-bet) |
| CO x BB, pote simples | CO abre 2,5 bb, BB paga | 5,5 bb / 97,5 bb | CO 26,7%; BB paga 24,9% |
| SB x BB, pote simples | SB abre 3 bb, BB paga | 6 bb / 97 bb | SB 44,5%; BB paga 48,5% |
| BB x BTN, pote 3-bet | BTN abre 2,5 bb, BB 3-bet 11 bb, BTN paga | 22,5 bb / 89 bb (SPR ~4) | BB 3-bet 10,0%; BTN paga 11,1% (4-bet com QQ+ e AK) |

O 3-bet do BB contra o BTN e a defesa por call do BB no BTN x BB 1.2.0 se completam (call + 3-bet ≤ 1 em cada mão). No SB x BB e no pote 3-bet o agressor pré-flop joga fora de posição (`preflop_aggressor` no spot), então o primeiro tema dele é "c-bet ou passar", e não "liderar". Os temas repetem entre spots (por exemplo, o BB liderando no flop existe no BTN x BB e no CO x BB): treino por tema, revisão por tema e progresso por tema ficam sempre dentro do spot. Um erro de uma versão antiga do BTN x BB volta na revisão como outra situação do mesmo tema no BTN x BB.

## Dados do usuário

`%LOCALAPPDATA%\dev.pokerlab.trainer\`: `pokerlab.sqlite3`, `packs\`, `backups\`, `exports\`. Binários em `%LOCALAPPDATA%\PokerLab\`. Atualizações não tocam nos dados. Recuperação: [docs/RECUPERACAO.md](docs/RECUPERACAO.md).

## Publicação (quando houver repositório)

Segredos esperados no ambiente `release` (só os nomes ficam no repositório): `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, variável `UPDATER_PUBKEY`; futuramente `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`. Uma tag `vX.Y.Z` cria a release como rascunho; valide instalador, `latest.json`, assinatura e notas antes de publicar.

## Licença

AGPL-3.0-or-later (ver `LICENSE` e `THIRD_PARTY_NOTICES.md`). Ranges do pacote são premissas de estudo próprias; a solução vale para a árvore modelada, sem rake nesta versão.
