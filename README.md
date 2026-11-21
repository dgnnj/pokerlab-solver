# PokerLab — treinador desktop de poker cash game (NLHE)

Aplicativo Windows 11 x64, em português, para treinar decisões pós-flop offline com estratégias e EVs calculados pelo [`postflop-solver`](https://github.com/b-inary/postflop-solver). Histórico local em SQLite, launcher com atualização assinada via GitHub Releases.

Especificação: `Especificacao_Treinador_Poker_Cash_Game.md` (v1.0). Decisões: [docs/adr/0001-fundacao.md](docs/adr/0001-fundacao.md).

## Estado (2026-11-20)

| Item | Estado |
|---|---|
| PoC do motor (river): solve, export versionado, recarga em outro processo, consulta | **Executado** |
| Testes de mapeamento combo/ação/frequência/EV, cartas bloqueadas, rake, perda de EV | **Executado** (28 testes Rust, 4 Vitest) |
| Pacote inicial BTN x BB SRP 100 bb (12 flops, 47 exercícios) | **Gerado e verificado** |
| App: launcher, treino, feedback por EV, estudo 13x13, revisão de erros, progresso, preset ~30 min, CSV, backup | **Executado** no app instalado (E2E via WebView2/CDP) |
| Persistência após fechar e reabrir | **Executado** (E2E no app instalado) |
| Instalador NSIS por usuário | **Executado** nesta máquina (Windows 11 Pro) |
| Updater 0.1.0 → 0.1.1 com feed local e chave dev, incl. falhas | **Executado** (11/11 checks) |
| CI (`ci.yml`) e release em rascunho (`release.yml`) | **Escrito, não executado** (não há repositório GitHub) |
| Teste em VM Windows 11 limpa, SmartScreen, Authenticode | **Pendente** |
| Publicação real, chave de produção, endpoint `OWNER/REPO` | **Pendente** (aguarda repositório e autorização) |

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

Pacote inicial (≈15 min, ~4 GB de RAM por flop; reaproveita cache quando a configuração não mudou):

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

## Estrutura

```text
crates/poker-core      domínio: cartas, spots, formato de solução, pacotes, avaliação de EV (sem o motor)
crates/solver-adapter  único módulo que usa o postflop-solver (build, solve, navegação, export)
crates/pokerlab-cli    solve / query / verify-reload / gen-pack / verify-pack
data/spots             spots versionados (TOML): PoC de river e template BTN x BB
data/packs             configuração do pacote inicial (lista de boards)
src-tauri              app Tauri: SQLite, treinador, updater, pacote incluído em resources/
src                    interface React
tests/e2e              testes ponta a ponta do app instalado e do updater
.github/workflows      CI e release em rascunho
```

## Dados do usuário

`%LOCALAPPDATA%\dev.pokerlab.trainer\`: `pokerlab.sqlite3`, `packs\`, `backups\`, `exports\`. Binários em `%LOCALAPPDATA%\PokerLab\`. Atualizações não tocam nos dados. Recuperação: [docs/RECUPERACAO.md](docs/RECUPERACAO.md).

## Publicação (quando houver repositório)

Segredos esperados no ambiente `release` (só os nomes ficam no repositório): `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, variável `UPDATER_PUBKEY`; futuramente `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`. Uma tag `vX.Y.Z` cria a release como rascunho; valide instalador, `latest.json`, assinatura e notas antes de publicar.

## Licença

AGPL-3.0-or-later (ver `LICENSE` e `THIRD_PARTY_NOTICES.md`). Ranges do pacote são premissas de estudo próprias; a solução vale para a árvore modelada, sem rake nesta versão.
