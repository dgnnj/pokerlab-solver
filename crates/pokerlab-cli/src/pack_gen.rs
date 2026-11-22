//! Pack generation: template spot + list of boards -> solved, validated, hashed pack.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};
use poker_core::cards::parse_board;
use poker_core::pack::{
    sha256_hex, ExerciseDef, PackFile, PackManifest, SolveSummary, PACK_SCHEMA_VERSION,
};
use poker_core::solution::{NodeSolution, SolutionFile, SolveStatus};
use poker_core::spot::SpotConfig;
use poker_core::texture::flop_texture_tags;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PackConfig {
    pub pack_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub license: String,
    pub provenance: String,
    /// Path of the template spot, relative to the pack config file.
    pub template_spot: String,
    pub boards: Vec<String>,
    #[serde(default = "default_depth")]
    pub export_depth: usize,
    /// Turn/river runouts to export (optional). When present, `export.depth` replaces
    /// `export_depth` for the initial street.
    #[serde(default)]
    pub export: Option<solver_adapter::ExportPlan>,
    /// Reject a solution whose final exploitability exceeds this % of the pot.
    pub max_exploitability_pct: f64,
}

fn default_depth() -> usize {
    3
}

/// Minimum joint reach (relative to the root) for a node to become an exercise.
pub const MIN_EXERCISE_REACH: f32 = 0.02;

/// Theme assignment from the node shape: street, how the previous street went, who acts and
/// whether they face a bet or a raise. Flop keys are unchanged from pack 1.0.0.
pub fn theme_for(node: &NodeSolution, spot: &SpotConfig) -> Option<(String, String, u8)> {
    if node.actions.len() < 2 {
        return None; // forced action, not a decision
    }
    if node.reach < MIN_EXERCISE_REACH {
        return None; // line almost never played in equilibrium
    }
    let t = poker_core::theme::classify(
        node.street,
        &node.line,
        node.to_call_chips,
        &node.position,
        spot,
    );
    Some((t.key, t.label, t.level))
}

pub fn exercises_for(sol: &SolutionFile) -> Vec<ExerciseDef> {
    let board = parse_board(&sol.spot.board).unwrap_or_default();
    let textures = flop_texture_tags(&board);
    sol.nodes
        .iter()
        .filter_map(|n| {
            let (theme, theme_label, level) = theme_for(n, &sol.spot)?;
            Some(ExerciseDef {
                id: format!("{}:{}", sol.solution_id, n.node_id),
                solution_id: sol.solution_id.clone(),
                node_id: n.node_id.clone(),
                theme,
                theme_label,
                street: n.street,
                position: n.position.clone(),
                pot_type: sol.spot.pot_type.clone(),
                board: sol.spot.board.clone(),
                textures: textures
                    .iter()
                    .cloned()
                    .chain(n.runout.iter().cloned())
                    .collect(),
                level,
            })
        })
        .collect()
}

pub fn spot_for_board(template: &SpotConfig, board: &str) -> Result<SpotConfig> {
    let mut s = template.clone();
    s.board = board.to_string();
    s.id = format!("{}_{}", template.id, board.to_lowercase());
    s.title = format!("{} — {}", template.title, board);
    s.validate().with_context(|| format!("board {board}"))?;
    Ok(s)
}

pub fn generate(config_path: &Path, out_dir: &Path, only: Option<usize>) -> Result<PackManifest> {
    let cfg_text = std::fs::read_to_string(config_path)?;
    let cfg: PackConfig = toml::from_str(&cfg_text).context("pack config")?;
    let base = config_path.parent().unwrap_or(Path::new("."));
    let template_path: PathBuf = base.join(&cfg.template_spot);
    let template = SpotConfig::from_toml(&std::fs::read_to_string(&template_path)?)
        .with_context(|| template_path.display().to_string())?;
    let sol_dir = out_dir.join("solutions");
    std::fs::create_dir_all(&sol_dir)?;

    let boards: Vec<&String> = cfg.boards.iter().take(only.unwrap_or(usize::MAX)).collect();
    let plan = cfg
        .export
        .clone()
        .unwrap_or_else(|| solver_adapter::ExportPlan::initial_only(cfg.export_depth));
    let plan_sig = serde_json::to_string(&plan)?;
    let mut files = Vec::new();
    let mut exercises = Vec::new();
    let mut solves = Vec::new();
    for (k, board) in boards.iter().enumerate() {
        let spot = spot_for_board(&template, board)?;
        let rel = format!("solutions/{}-v{}.json.gz", spot.id, spot.version);
        let path = out_dir.join(&rel);
        let hash = spot.config_hash(solver_adapter::ENGINE_REV);
        // Cache: reuse an existing solution with identical canonical config, engine revision
        // and export plan.
        let sol = match SolutionFile::load(&path) {
            Ok(s)
                if s.config_hash == hash && s.export_plan.as_deref() == Some(plan_sig.as_str()) =>
            {
                eprintln!(
                    "[{}/{}] {board}: cache válido, reutilizando",
                    k + 1,
                    boards.len()
                );
                s
            }
            _ => {
                eprintln!("[{}/{}] {board}: resolvendo...", k + 1, boards.len());
                let cancel = AtomicBool::new(false);
                let mut solved = solver_adapter::solve_spot(&spot, &cancel, |p| {
                    if p.iteration % 100 == 0 {
                        eprintln!(
                            "    it {:>4}/{} explorabilidade {:.3}% do pote",
                            p.iteration,
                            p.max_iterations,
                            100.0 * p.exploitability_chips as f64 / spot.starting_pot_chips as f64
                        );
                    }
                })?;
                let s = solver_adapter::build_solution_file_with_plan(&mut solved, &spot, &plan)?;
                s.save(&path)?;
                s
            }
        };
        let pct = sol.solve.final_exploitability_pct_pot;
        if pct > cfg.max_exploitability_pct {
            bail!(
                "{board}: explorabilidade {pct:.3}% acima do limite {}%",
                cfg.max_exploitability_pct
            );
        }
        if sol.solve.status != SolveStatus::Converged {
            eprintln!("    aviso: {board} parou no limite de iterações ({pct:.3}% do pote)");
        }
        // Reload from disk to check the persisted file round-trips.
        let reloaded = SolutionFile::load(&path)?;
        if reloaded != sol {
            bail!("{board}: arquivo recarregado difere do gerado");
        }
        let bytes = std::fs::read(&path)?;
        eprintln!(
            "    ok: {:.3}% do pote, {:.1}s, {:.0} MiB estimados, {} nós, {:.0} KiB",
            pct,
            sol.solve.solve_ms as f64 / 1000.0,
            sol.solve.memory_estimate_bytes as f64 / 1048576.0,
            sol.nodes.len(),
            bytes.len() as f64 / 1024.0
        );
        files.push(PackFile {
            path: rel,
            solution_id: sol.solution_id.clone(),
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        });
        exercises.extend(exercises_for(&sol));
        solves.push(SolveSummary {
            solution_id: sol.solution_id.clone(),
            board: sol.spot.board.clone(),
            final_exploitability_pct_pot: pct,
            solve_ms: sol.solve.solve_ms,
            memory_estimate_bytes: sol.solve.memory_estimate_bytes,
            status: format!("{:?}", sol.solve.status),
        });
    }
    let manifest = PackManifest {
        schema_version: PACK_SCHEMA_VERSION,
        pack_id: cfg.pack_id,
        version: cfg.version,
        title: cfg.title,
        description: cfg.description,
        license: cfg.license,
        provenance: cfg.provenance,
        created_at: poker_core::now_iso8601(),
        engine: solver_adapter::engine_info(),
        files,
        exercises,
        solves,
    };
    let json = serde_json::to_vec_pretty(&manifest)?;
    poker_core::solution::write_atomic(&out_dir.join("manifest.json"), &json)?;
    // Final integrity pass exactly as the app does it.
    let m = PackManifest::load(out_dir)?;
    m.verify_and_load(out_dir)?;
    Ok(manifest)
}
