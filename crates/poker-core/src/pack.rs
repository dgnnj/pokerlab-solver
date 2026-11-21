//! Solution packs: a manifest with provenance, per-file SHA-256 and the list of exercises.
//!
//! Layout on disk:
//! ```text
//! <pack_dir>/manifest.json
//! <pack_dir>/solutions/<solution_id>.json.gz
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::solution::{EngineInfo, SolutionFile};
use crate::spot::Street;
use crate::CoreError;

pub const PACK_SCHEMA_VERSION: u32 = 1;
/// Refuse absurd pack files (corruption / wrong file) before reading them.
pub const MAX_PACK_FILE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackFile {
    pub path: String,
    pub solution_id: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExerciseDef {
    pub id: String,
    pub solution_id: String,
    pub node_id: String,
    pub theme: String,
    pub theme_label: String,
    pub street: Street,
    pub position: String,
    pub pot_type: String,
    pub board: String,
    pub textures: Vec<String>,
    /// 1 = basic, 2 = intermediate, 3 = advanced.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveSummary {
    pub solution_id: String,
    pub board: String,
    pub final_exploitability_pct_pot: f64,
    pub solve_ms: u64,
    pub memory_estimate_bytes: u64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackManifest {
    pub schema_version: u32,
    pub pack_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub license: String,
    pub provenance: String,
    pub created_at: String,
    pub engine: EngineInfo,
    pub files: Vec<PackFile>,
    pub exercises: Vec<ExerciseDef>,
    pub solves: Vec<SolveSummary>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl PackManifest {
    pub fn load(pack_dir: &Path) -> Result<Self, CoreError> {
        let p = pack_dir.join("manifest.json");
        let raw = std::fs::read(&p).map_err(|e| CoreError::Io(format!("{}: {e}", p.display())))?;
        let m: PackManifest =
            serde_json::from_slice(&raw).map_err(|e| CoreError::InvalidPack(e.to_string()))?;
        if m.schema_version != PACK_SCHEMA_VERSION {
            return Err(CoreError::InvalidPack(format!(
                "schema de pacote {} não suportado",
                m.schema_version
            )));
        }
        Ok(m)
    }

    pub fn file_path(&self, pack_dir: &Path, f: &PackFile) -> Result<PathBuf, CoreError> {
        // Reject absolute paths and traversal: pack files must stay inside the pack dir.
        let rel = Path::new(&f.path);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(CoreError::InvalidPack(format!(
                "caminho inválido: {}",
                f.path
            )));
        }
        Ok(pack_dir.join(rel))
    }

    /// Verifies size and hash of every file, parses and validates each solution, and checks
    /// that every exercise points to an existing node.
    pub fn verify_and_load(&self, pack_dir: &Path) -> Result<Vec<SolutionFile>, CoreError> {
        let mut out = Vec::with_capacity(self.files.len());
        for f in &self.files {
            let path = self.file_path(pack_dir, f)?;
            let meta = std::fs::metadata(&path)
                .map_err(|e| CoreError::InvalidPack(format!("{}: {e}", f.path)))?;
            if meta.len() != f.bytes || meta.len() > MAX_PACK_FILE_BYTES {
                return Err(CoreError::InvalidPack(format!(
                    "{}: tamanho {} difere do manifesto ({})",
                    f.path,
                    meta.len(),
                    f.bytes
                )));
            }
            let raw = std::fs::read(&path).map_err(|e| CoreError::Io(e.to_string()))?;
            let h = sha256_hex(&raw);
            if h != f.sha256 {
                return Err(CoreError::InvalidPack(format!(
                    "{}: hash SHA-256 não confere",
                    f.path
                )));
            }
            let sol = SolutionFile::load(&path)?;
            if sol.solution_id != f.solution_id {
                return Err(CoreError::InvalidPack(format!(
                    "{}: solution_id divergente",
                    f.path
                )));
            }
            out.push(sol);
        }
        for ex in &self.exercises {
            let sol = out
                .iter()
                .find(|s| s.solution_id == ex.solution_id)
                .ok_or_else(|| {
                    CoreError::InvalidPack(format!("exercício {} sem solução", ex.id))
                })?;
            if sol.node(&ex.node_id).is_none() {
                return Err(CoreError::InvalidPack(format!(
                    "exercício {} sem nó {}",
                    ex.id, ex.node_id
                )));
            }
        }
        Ok(out)
    }
}
