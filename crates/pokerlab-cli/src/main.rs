//! `pokerlab-cli`: development commands for the PokerLab trainer.
//!
//! ```text
//! pokerlab-cli solve data/spots/river_poc.toml --out out/river_poc.solution.json --probe out/river_poc.probe.json
//! pokerlab-cli verify-reload out/river_poc.solution.json out/river_poc.probe.json
//! pokerlab-cli query out/river_poc.solution.json --node root --combo AhKd --action 0
//! pokerlab-cli gen-pack data/packs/btn_vs_bb_srp_100bb.toml --out src-tauri/resources/packs/btn-vs-bb-srp-100bb
//! pokerlab-cli verify-pack src-tauri/resources/packs/btn-vs-bb-srp-100bb
//! ```

mod pack_gen;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use poker_core::cards::Combo;
use poker_core::eval::{evaluate, Tolerance};
use poker_core::fmt_bb;
use poker_core::pack::PackManifest;
use poker_core::solution::SolutionFile;
use poker_core::spot::SpotConfig;
use solver_adapter::Probe;

#[derive(Parser)]
#[command(
    name = "pokerlab-cli",
    version,
    about = "Ferramentas de desenvolvimento do PokerLab"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Configura, resolve e exporta um spot (TOML) para o formato versionado.
    Solve {
        spot: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Grava valores lidos diretamente do motor (precisão total) para checar a recarga.
        #[arg(long)]
        probe: Option<PathBuf>,
        /// Combos da sonda no nó raiz (separados por vírgula). Padrão: primeiro combo elegível.
        #[arg(long)]
        probe_combos: Option<String>,
        #[arg(long, default_value_t = 4)]
        depth: usize,
    },
    /// Recarrega uma solução em processo separado e compara com a sonda do motor.
    VerifyReload {
        solution: PathBuf,
        probe: PathBuf,
        #[arg(long, default_value_t = 1e-4)]
        freq_tol: f32,
        /// Tolerância de EV em fichas.
        #[arg(long, default_value_t = 0.01)]
        ev_tol: f32,
    },
    /// Consulta uma decisão salva: ações, frequências e EVs de um combo; avalia uma ação.
    Query {
        solution: PathBuf,
        #[arg(long, default_value = "root")]
        node: String,
        #[arg(long)]
        combo: Option<String>,
        #[arg(long)]
        action: Option<usize>,
    },
    /// Gera o pacote de exercícios a partir de um template de spot e uma lista de boards.
    GenPack {
        config: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Limita aos N primeiros boards (para testes rápidos).
        #[arg(long)]
        only: Option<usize>,
    },
    /// Verifica hashes, tamanhos e conteúdo de um pacote.
    VerifyPack { dir: PathBuf },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Solve {
            spot,
            out,
            probe,
            probe_combos,
            depth,
        } => solve(spot, out, probe, probe_combos, depth),
        Cmd::VerifyReload {
            solution,
            probe,
            freq_tol,
            ev_tol,
        } => verify_reload(solution, probe, freq_tol, ev_tol),
        Cmd::Query {
            solution,
            node,
            combo,
            action,
        } => query(solution, node, combo, action),
        Cmd::GenPack { config, out, only } => {
            let m = pack_gen::generate(&config, &out, only)?;
            println!(
                "pacote {} v{}: {} soluções, {} exercícios em {}",
                m.pack_id,
                m.version,
                m.files.len(),
                m.exercises.len(),
                out.display()
            );
            Ok(())
        }
        Cmd::VerifyPack { dir } => {
            let m = PackManifest::load(&dir)?;
            let sols = m.verify_and_load(&dir)?;
            let nodes: usize = sols.iter().map(|s| s.nodes.len()).sum();
            println!(
                "OK: pacote {} v{} — {} arquivos íntegros, {} nós, {} exercícios",
                m.pack_id,
                m.version,
                m.files.len(),
                nodes,
                m.exercises.len()
            );
            Ok(())
        }
    }
}

fn solve(
    spot_path: PathBuf,
    out: PathBuf,
    probe_path: Option<PathBuf>,
    probe_combos: Option<String>,
    depth: usize,
) -> Result<()> {
    let spot = SpotConfig::from_toml(&std::fs::read_to_string(&spot_path)?)
        .with_context(|| spot_path.display().to_string())?;
    let cpb = spot.chips_per_bb;
    println!("== Spot {} (v{}) — {}", spot.id, spot.version, spot.title);
    println!(
        "Board: {}  | rua inicial: {:?}",
        spot.board, spot.initial_street
    );
    println!("OOP = {} | IP = {}", spot.oop_position, spot.ip_position);
    println!(
        "Range OOP ({}): {}",
        spot.oop_range.source, spot.oop_range.text
    );
    println!(
        "Range IP  ({}): {}",
        spot.ip_range.source, spot.ip_range.text
    );
    println!(
        "Pote: {} bb ({} fichas) | stack efetivo: {} bb ({} fichas) | escala: {} fichas/bb",
        fmt_bb(spot.starting_pot_chips as f64, cpb),
        spot.starting_pot_chips,
        fmt_bb(spot.effective_stack_chips as f64, cpb),
        spot.effective_stack_chips,
        cpb
    );
    println!(
        "Rake: {:.2}% cap {} fichas ({})",
        spot.rake.rate * 100.0,
        spot.rake.cap_chips,
        spot.rake.structure
    );
    let t = &spot.tree;
    println!(
        "Árvore: river OOP bet [{}] raise [{}] | IP bet [{}] raise [{}] | add_allin {} force_allin {} merge {}",
        t.oop.river.bet, t.oop.river.raise, t.ip.river.bet, t.ip.river.raise,
        t.add_allin_threshold, t.force_allin_threshold, t.merging_threshold
    );

    let game = solver_adapter::build_game(&spot)?;
    let (m, mc) = game.memory_usage();
    println!(
        "Memória estimada: {:.1} MiB (comprimido: {:.1} MiB)",
        m as f64 / 1048576.0,
        mc as f64 / 1048576.0
    );
    drop(game);

    let cancel = AtomicBool::new(false);
    let mut solved = solver_adapter::solve_spot(&spot, &cancel, |p| {
        if p.iteration % 50 == 0 {
            println!(
                "  iteração {:>4}/{}: explorabilidade {:.4} fichas ({:.3}% do pote)",
                p.iteration,
                p.max_iterations,
                p.exploitability_chips,
                100.0 * p.exploitability_chips as f64 / spot.starting_pot_chips as f64
            );
        }
    })?;
    let info = solved.info.clone();
    println!(
        "Resolvido: {:?} | explorabilidade final {:.4} fichas = {:.3}% do pote (meta {:.3} fichas) | {} ms | {} threads | comprimido: {}",
        info.status, info.final_exploitability_chips, info.final_exploitability_pct_pot,
        info.target_exploitability_chips, info.solve_ms, info.threads, info.compressed
    );

    let file = solver_adapter::build_solution_file(&mut solved, &spot, depth)?;
    println!(
        "Ações na raiz: {}",
        file.root_actions
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    );
    println!("Nós exportados ({}):", file.nodes.len());
    for n in &file.nodes {
        let freqs = n.aggregate_frequencies();
        println!(
            "  [{}] {} ({}) alcance {:.1}%, pote {} bb, {} combos: {}",
            n.node_id,
            if n.line.is_empty() {
                "início".to_string()
            } else {
                n.line.join(", ")
            },
            n.position,
            n.reach * 100.0,
            fmt_bb(n.pot_chips as f64, cpb),
            n.combos.len(),
            n.actions
                .iter()
                .zip(&freqs)
                .map(|(a, f)| format!("{} {:.1}%", a.label, f * 100.0))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
    file.save(&out)?;
    let size = std::fs::metadata(&out)?.len();
    println!(
        "Solução salva em {} ({} bytes, schema v{}, hash {})",
        out.display(),
        size,
        file.schema_version,
        &file.config_hash[..16]
    );

    let root = file.node("root").context("sem nó raiz")?;
    let combos: Vec<Combo> = match probe_combos {
        Some(s) => s.split(',').map(Combo::parse).collect::<Result<_, _>>()?,
        None => vec![Combo::parse(&root.combos[0])?],
    };
    let mut probes = Vec::new();
    for c in &combos {
        let p = solver_adapter::probe_live(
            &mut solved.game,
            &spot,
            &file.solution_id,
            &root.history,
            "root",
            c,
        )?;
        print_decision(&p);
        probes.push(p);
    }
    if let Some(pp) = probe_path {
        poker_core::solution::write_atomic(&pp, &serde_json::to_vec_pretty(&probes)?)?;
        println!("Sonda do motor salva em {}", pp.display());
    }
    Ok(())
}

fn print_decision(p: &Probe) {
    println!("Decisão no nó '{}' com {}:", p.node_id, p.combo);
    for i in 0..p.actions.len() {
        println!(
            "   {:<36} freq {:>7.3}%  EV {:>10.4} fichas",
            p.actions[i],
            p.frequencies[i] * 100.0,
            p.evs_chips[i]
        );
    }
}

fn verify_reload(solution: PathBuf, probe: PathBuf, freq_tol: f32, ev_tol: f32) -> Result<()> {
    let sol = SolutionFile::load(&solution)?;
    let probes: Vec<Probe> = serde_json::from_slice(&std::fs::read(&probe)?)?;
    println!(
        "Solução {} recarregada (schema v{}, motor {} @ {})",
        sol.solution_id,
        sol.schema_version,
        sol.engine.name,
        &sol.engine.git_rev[..10]
    );
    let mut worst_f = 0f32;
    let mut worst_ev = 0f32;
    for p in &probes {
        if p.solution_id != sol.solution_id {
            bail!("sonda de outra solução: {}", p.solution_id);
        }
        let node = sol.node(&p.node_id).context("nó ausente")?;
        let combo = Combo::parse(&p.combo)?;
        let ci = node
            .combo_index(&combo)
            .context("combo ausente no arquivo")?;
        let labels: Vec<&str> = node.actions.iter().map(|a| a.label.as_str()).collect();
        if labels != p.actions.iter().map(|s| s.as_str()).collect::<Vec<_>>() {
            bail!("ações divergentes: {labels:?} vs {:?}", p.actions);
        }
        for j in 0..node.actions.len() {
            let df = (node.strategy[j][ci] - p.frequencies[j]).abs();
            let de = (node.ev[j][ci] - p.evs_chips[j]).abs();
            worst_f = worst_f.max(df);
            worst_ev = worst_ev.max(de);
            println!(
                "  {} | {:<34} freq arquivo {:.5} motor {:.5} | EV arquivo {:.3} motor {:.4}",
                p.combo,
                labels[j],
                node.strategy[j][ci],
                p.frequencies[j],
                node.ev[j][ci],
                p.evs_chips[j]
            );
        }
    }
    println!("Maior diferença: frequência {worst_f:.2e} (tol {freq_tol:.0e}), EV {worst_ev:.2e} fichas (tol {ev_tol:.0e})");
    if worst_f > freq_tol || worst_ev > ev_tol {
        bail!("FALHA: recarga fora da tolerância");
    }
    println!("OK: mesma decisão, mesmos valores dentro da tolerância");
    Ok(())
}

fn query(
    solution: PathBuf,
    node_id: String,
    combo: Option<String>,
    action: Option<usize>,
) -> Result<()> {
    let sol = SolutionFile::load(&solution)?;
    let cpb = sol.spot.chips_per_bb;
    let node = sol.node(&node_id).with_context(|| {
        format!(
            "nó '{node_id}' não existe; disponíveis: {}",
            sol.nodes
                .iter()
                .map(|n| n.node_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    println!(
        "Nó '{}' — {} decide, board {}, pote {} bb, a pagar {} bb",
        node.node_id,
        node.position,
        node.board.join(""),
        fmt_bb(node.pot_chips as f64, cpb),
        fmt_bb(node.to_call_chips as f64, cpb)
    );
    let combo = match combo {
        Some(c) => Combo::parse(&c)?,
        None => Combo::parse(&node.combos[0])?,
    };
    let ci = node
        .combo_index(&combo)
        .with_context(|| format!("{combo} sem peso neste nó (bloqueado ou fora do range)"))?;
    println!(
        "Combo {} (peso {:.3}, equidade {:.1}%)",
        combo,
        node.weights[ci],
        node.equity[ci] * 100.0
    );
    for (j, a) in node.actions.iter().enumerate() {
        println!(
            "   [{j}] {:<34} freq {:>7.3}%  EV {:>8.3} bb",
            a.label,
            node.strategy[j][ci] * 100.0,
            node.ev[j][ci] as f64 / cpb as f64
        );
    }
    if let Some(a) = action {
        let fb = evaluate(node, &combo, a, cpb, Tolerance::default())?;
        println!(
            "Escolha [{}] {}: {} | perda {:.4} bb ({:.2}% do pote) | melhor: {} | tolerância {:.3} bb",
            a,
            node.actions[a].label,
            fb.grade.label_pt(),
            fb.loss_bb,
            fb.loss_pct_pot,
            node.actions[fb.best_index].label,
            fb.tolerance_chips / cpb as f64
        );
    }
    Ok(())
}
