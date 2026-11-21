# ADR 0001 — Fundação do PokerLab

- **Status:** aceito
- **Data:** 2026-11-20
- **Contexto:** `Especificacao_Treinador_Poker_Cash_Game.md` v1.0. Workspace vazio; Windows 11 x64; Ryzen 5 7600X, 32 GB.

## Decisões

### 1. Framework desktop: Tauri 2 + Rust + TypeScript/React + SQLite
- Tauri 2.12 (crate `tauri` 2.x, CLI `@tauri-apps/cli` 2.12.6). Tauri 3 existe só em alpha e foi descartado.
- React 19, Vite 8, TypeScript 5.9 (o TS 7 nativo é recente demais para fixar agora).
- Interface empacotada no executável e renderizada pelo WebView2 do sistema. Sem servidor local.
- Comandos Tauri tipados; cada comando tem permissão própria (`allow-<comando>`) concedida explicitamente em `src-tauri/capabilities/default.json` (`build.rs` declara o `AppManifest`).
- O cálculo pesado (solver) **não** roda no app nesta versão: o treino consulta pacotes pré-calculados. O solver fica no CLI de desenvolvimento.

### 2. Motor: `postflop-solver` fixado por commit
- Revisão `9d1509fe5077d019825f833eed04b16d342dfda1` (último commit, 2023-10-01; projeto com desenvolvimento suspenso).
- `default-features = false, features = ["rayon"]`: sem `bincode`/`zstd`. A dependência `bincode = 2.0.0-rc.3` do motor resolveria para 2.x estável, cuja API mudou; como usamos formato próprio, a feature é desnecessária.
- Toda a API externa fica em `crates/solver-adapter`. `poker-core` não depende do motor.
- Solve com `solve_step` + `compute_exploitability` a cada 10 iterações + `finalize`, para ter progresso e cancelamento cooperativo (tarefa cancelada não publica resultado).
- Perfil `dev` compila o motor com `opt-level = 3` (testes de integração usam o motor real).

### 3. Formato de solução e pacotes (schema v1)
- JSON (gzip para pacotes), independente do binário interno do motor: spot completo, info do motor, parâmetros e explorabilidade final, e por nó: combos elegíveis (peso normalizado > 0 e não bloqueados pelo board), estratégia `[ação][combo]`, EV `[ação][combo]`, equidade, alcance relativo.
- Fichas inteiras com escala explícita: **100 fichas = 1 bb**. Conversão para bb só na exibição.
- EV na convenção do motor: desistir = 0, todas as ações do mesmo nó com a mesma base. Perda de EV = maior EV − EV escolhido, mesmo nó e mesmo combo.
- `serde_json` com `float_roundtrip` para recarga bit-a-bit dos `f32`.
- Arredondamento no armazenamento: frequência 1e-5, EV 1e-3 ficha.
- Pacote = `manifest.json` (versão, licença, proveniência, motor, SHA-256 e tamanho por arquivo, exercícios, resumo de cada solve) + `solutions/*.json.gz`. O app valida tamanho, hash e conteúdo antes de registrar; cópia instalada corrompida é substituída pela do instalador.
- Cache de geração: chave = SHA-256 da configuração canônica (inclui ranges, tamanhos, rake, parâmetros de solve) + revisão do motor. Escrita temporária + rename atômico.

### 4. Pacote inicial
- BTN x BB, SRP, 100 bb, blinds 0,5/1, BTN abre 2,5 bb, SB desiste, BB paga: pote 5,5 bb, stack efetivo 97,5 bb.
- **Sem rake** nesta versão (referência). A estrutura da sala ainda não foi identificada; ajustar `[rake]` no template e regenerar.
- Ranges construídos manualmente e marcados como **premissa de estudo** (`manual_premise = true`).
- Árvore: flop BB só check/check-raise 3x; BTN 33%/75%, raise 3x; turn/river 75%, raise 3x; add all-in 1,5; force all-in 0,15; merge 0,1.
- Meta: 0,5% do pote inicial, máx. 1000 iterações; o gerador recusa > 1,0%.
- Exercícios: nós de decisão do flop até profundidade 3, com ≥ 2 ações e alcance conjunto ≥ 2% da raiz.
- **Turn e river (pacote 1.1.0, 2026-11-21):** exportar todos os runouts não cabe no instalador. O plano `[export]` segue linhas principais (flop x-x, x-bS-c, x-bL-c; turn x-x, x-bS-c) e cartas representativas escolhidas de forma determinística por categoria (`poker_core::runout`): carta do naipe do board (3ª/4ª do naipe), overcard, pareia a carta alta, baixa, do meio. Cada exercício usa os ranges do motor na linha jogada (pesos normalizados no nó). A escolha das cartas é cobertura, não frequência real: as estatísticas de turn/river não representam a distribuição completa de runouts. IDs de nó usam `|` entre ruas, p.ex. `x-b182-c|Th|x`.

- **Mão completa (2026-11-21):** a exceção ao "solver não roda no app". Exportar todos os runouts não cabe no instalador, então o app resolve a árvore completa do flop sob demanda, numa thread própria, com o `spot` gravado na solução do pacote (o hash de configuração é comparado e exibido). Uma árvore por vez; trocar de board cancela/descarta a anterior. Limite de 6 GiB estimados sem compressão (todos os 12 flops cabem: 1,2–4,2 GB), acima disso armazenamento comprimido. O app depende do motor só via `solver-adapter` (que reexporta `PostFlopGame`). Alternativa descartada: re-resolver turn/river a partir dos ranges da linha (mais leve, mas não é a política do motor na árvore completa).

### 5. Avaliação
- Tolerância "próxima": `max(0,05 bb, 1% do pote antes da decisão)` — da ordem do ruído da meta de explorabilidade. Faixas: melhor / próxima / imprecisão (≤ 5% do pote) / erro.
- Amostragem `representative` (exercício uniforme no tema; combo pelo peso real no nó), `hard` (pondera estratégias mistas) e `review`. Estatísticas separadas por modo.

### 6. Armazenamento local
- `%LOCALAPPDATA%\dev.pokerlab.trainer\` (API `app_local_data_dir`): `pokerlab.sqlite3`, `packs/<id>/<versão>/`, `backups/`, `exports/`. O instalador/updater não toca nessa pasta.
- SQLite (rusqlite 0.37, bundled): migrations por `PRAGMA user_version`, transações, FKs; backup pela API de backup antes de migração de banco existente; banco mais novo que o app é recusado.
- Uma tentativa por exercício apresentado (`attempts.token UNIQUE`).

### 7. Distribuição e atualização
- NSIS por usuário (`installMode: currentUser`), pt-BR, WebView2 via bootstrapper silencioso.
- Launcher = primeira tela do app. `tauri-plugin-updater` 2.14 com `latest.json` do GitHub Releases, instalação `passive`. Instância única via `tauri-plugin-single-instance`.
- Enquanto o endpoint contiver `OWNER/REPO`, o canal aparece como "não configurado" e nada é consultado.
- **Chaves:** a chave pública em `tauri.conf.json` é de **desenvolvimento** (`.keys/`, fora do git). Releases usam `scripts/release-config.mjs` para injetar o endpoint real e a chave pública de produção (`vars.UPDATER_PUBKEY`); a privada fica em `secrets.TAURI_SIGNING_PRIVATE_KEY`.
- Ensaio local: `tauri.updater-test.conf.json` (identificador separado, feed HTTP em 127.0.0.1 com `dangerousInsecureTransportProtocol`, só para teste).
- Releases criadas como rascunho por `release.yml` para tags `vX.Y.Z`; versão coerente checada por `scripts/check-version.mjs`. Actions de terceiros fixadas por SHA.

### 8. Licença
- O motor é AGPL-3.0-or-later; o projeto combinado é publicado sob **AGPL-3.0-or-later** (`LICENSE`), com `THIRD_PARTY_NOTICES.md` no instalador e fonte correspondente por tag.

### 9. Ambiente de build
- O repositório está no OneDrive. Use `CARGO_TARGET_DIR` fora dele (ex.: `%LOCALAPPDATA%\pokerlab-target`) para não sincronizar GBs de artefatos.

## Consequências
- Atualizar o motor exige: nova revisão em `Cargo.toml` e `ENGINE_REV`, regenerar pacotes (hash muda), rodar os testes de integração e comparar EVs.
- Trocar ranges/rake/tamanhos muda o `config_hash` e invalida o cache automaticamente.
