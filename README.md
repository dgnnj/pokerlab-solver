# PokerLab — treinador desktop de poker cash game (NLHE)

Aplicativo Windows 11 x64, em português, para treinar decisões pós-flop offline com estratégias e EVs calculados pelo [`postflop-solver`](https://github.com/b-inary/postflop-solver). Histórico local em SQLite, launcher com atualização assinada via GitHub Releases.

Especificação: `Especificacao_Treinador_Poker_Cash_Game.md` (v1.0). Decisões: [docs/adr/0001-fundacao.md](docs/adr/0001-fundacao.md).

## Estado (2026-11-21)

| Item | Estado |
|---|---|
| PoC do motor (river): solve, export versionado, recarga em outro processo, consulta | **Executado** |
| Testes de mapeamento combo/ação/frequência/EV, cartas bloqueadas, rake, perda de EV | **Executado** (50 testes Rust, 6 Vitest) |
| Pacote BTN x BB SRP 100 bb v1.1.0: 12 flops, flop + turn + river em linhas principais com cartas representativas | **Gerado e verificado** |
| App: launcher, treino, feedback por EV, estudo 13x13, revisão de erros, progresso, preset ~30 min, CSV, backup | **Executado** no app instalado (E2E via WebView2/CDP) |
| Persistência após fechar e reabrir | **Executado** (E2E no app instalado) |
| Instalador NSIS por usuário | **Executado** nesta máquina (Windows 11 Pro) |
| Updater 0.1.0 → 0.1.1 com feed local e chave dev, incl. falhas | **Executado** (11/11 checks) |
| Updater real 0.3.0 → 0.3.1 pelo feed público (chave de produção), com adiar e histórico preservado | **Executado** nesta máquina (8/8 passos; `%LOCALAPPDATA%` virtualizado, não substitui a VM limpa) |
| CI (`ci.yml`) e release (`release.yml`) no GitHub | **Executado** (releases v0.1.0 a v0.3.1 publicadas) |
| Teste em VM Windows 11 limpa, SmartScreen, Authenticode | **Pendente** |
| Publicação com chave de produção e feed `dgnnj/pokerlab-solver` | **Executado** (v0.3.1 é a Latest; `latest.json` assinado e verificado contra a chave pública de produção) |
| Simulação de mão completa (flop ao river contra a política do motor), árvore do flop resolvida no app | **Executado** no build release (E2E via WebView2: 9h6h3h como BB e 8s7s5d como BTN, 3 mãos cada, histórico após reabrir); publicada na v0.3.0; exportação das mãos em CSV publicada na v0.3.1 (E2E rodou no build release, não no instalador baixado do feed) |

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

Pacote inicial (≈15 min, ~4 GB de RAM por flop; reaproveita cache quando spot, motor e plano de exportação não mudaram). O plano `[export]` em `data/packs/btn_vs_bb_srp_100bb.toml` escolhe as linhas de flop/turn e quantas cartas representativas de turn e river exportar:

```bash
cargo run --release -p pokerlab-cli -- gen-pack data/packs/btn_vs_bb_srp_100bb.toml --out src-tauri/resources/packs/btn-vs-bb-srp-100bb
```

```bash
cargo run --release -p pokerlab-cli -- verify-pack src-tauri/resources/packs/btn-vs-bb-srp-100bb
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
node tests/e2e/hand-sim.mjs --exe "$LOCALAPPDATA/pokerlab-target/release/pokerlab.exe" --pack src-tauri/resources/packs/btn-vs-bb-srp-100bb --board 9h6h3h --hands 3 --hero BB
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

Atualização real pelo feed público, depois de publicar uma release (instala a anterior em `%LOCALAPPDATA%\PokerLab`; dados em `--data`):

```bash
gh release download v0.3.0 -p 'PokerLab_0.3.0_x64-setup.exe' -D out/updater-live
node tests/e2e/updater-live.mjs --setup out/updater-live/PokerLab_0.3.0_x64-setup.exe --from 0.3.0 --to 0.3.1
```

## Estrutura

```text
crates/poker-core      domínio: cartas, spots, formato de solução, pacotes, avaliação de EV, ranking de mãos (sem o motor)
crates/solver-adapter  único módulo que usa o postflop-solver (build, solve, navegação, export, simulação de mão)
crates/pokerlab-cli    solve / query / verify-reload / gen-pack / verify-pack
data/spots             spots versionados (TOML): PoC de river e template BTN x BB
data/packs             configuração do pacote inicial (lista de boards)
src-tauri              app Tauri: SQLite, treinador, updater, pacote incluído em resources/
src                    interface React
tests/e2e              testes ponta a ponta do app instalado e do updater
.github/workflows      CI e release em rascunho
```

## Mão completa

O pacote só guarda as linhas principais, então a mão inteira usa a árvore completa do flop, resolvida no próprio app em segundo plano com o spot gravado na solução do pacote (mesmo hash de configuração; o app mostra se difere). No Ryzen 5 7600X: 25–60 s e 1,2–4,2 GB de RAM por board (acima de 6 GiB estimados o motor usa armazenamento comprimido). Uma árvore por vez fica na memória enquanto o app está aberto; as mãos seguintes no mesmo board são instantâneas.

- Mãos privadas sorteadas juntas pelos pesos dos ranges, sem cartas em comum; o adversário sorteia cada ação pela estratégia da mão dele no nó; turn e river saem do baralho restante. All-in pago antes do river é nó terminal no motor (ele faz a média dos runouts), então o app distribui as cartas restantes para o showdown.
- Cada decisão é avaliada pela perda de EV no mesmo nó e mão. Se a sua linha leva a mão a peso praticamente nulo na solução (< 0,001% do peso inicial), a nota termina na última decisão avaliável e o motivo aparece.
- Resultado no pós-flop (o pote inicial conta para quem leva) separado da qualidade das decisões. Histórico em `hands` / `hand_decisions` (schema v2; migração com backup).
- Exportar mãos completas (CSV), no histórico de mãos ou em Configurações: grava `exports\pokerlab-maos-<timestamp>.csv` com uma linha por decisão do aluno e os dados da mão repetidos em cada linha (cartas dos dois, board final, linha, vencedor, resultado, nota, EVs, frequência, equidade, motivo de corte). Mão interrompida antes da primeira decisão sai com uma linha e as colunas da decisão vazias; `ended_at` vazio indica mão interrompida.

## Dados do usuário

`%LOCALAPPDATA%\dev.pokerlab.trainer\`: `pokerlab.sqlite3`, `packs\`, `backups\`, `exports\`. Binários em `%LOCALAPPDATA%\PokerLab\`. Atualizações não tocam nos dados. Recuperação: [docs/RECUPERACAO.md](docs/RECUPERACAO.md).

## Publicação (quando houver repositório)

Segredos esperados no ambiente `release` (só os nomes ficam no repositório): `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, variável `UPDATER_PUBKEY`; futuramente `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`. Uma tag `vX.Y.Z` cria a release como rascunho; valide instalador, `latest.json`, assinatura e notas antes de publicar.

## Licença

AGPL-3.0-or-later (ver `LICENSE` e `THIRD_PARTY_NOTICES.md`). Ranges do pacote são premissas de estudo próprias; a solução vale para a árvore modelada, sem rake nesta versão.
