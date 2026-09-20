//! Curated model catalog and the machine-fit recommendation.

use serde::Serialize;

/// Machine profile that drives recommendation and fit labels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineProfile {
    pub total_ram_bytes: u64,
    pub cpu: String,
    pub accelerator: String,
    pub free_disk_bytes: u64,
    pub process_arch: String,
    pub os_arch: String,
}

impl MachineProfile {
    pub fn detect(data_dir: &std::path::Path) -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
        let cpu = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let free_disk_bytes = sysinfo::Disks::new_with_refreshed_list()
            .iter()
            .filter(|d| data_dir.starts_with(d.mount_point()))
            .map(|d| d.available_space())
            .max()
            .unwrap_or(0);
        let (process_arch, os_arch) = host_arch_labels();
        Self {
            total_ram_bytes: sys.total_memory(),
            cpu,
            accelerator: super::pin::preferred_engine_pin()
                .map(|pin| pin.accelerator.to_string())
                .unwrap_or_else(|_| "none".into()),
            free_disk_bytes,
            process_arch,
            os_arch,
        }
    }

    /// RAM the model may use, leaving headroom for the OS, Larra, Shelves,
    /// search and OCR.
    pub fn model_budget_bytes(&self) -> u64 {
        (self.total_ram_bytes as f64 * 0.65) as u64
    }

    /// Estimated runtime need for a file of `file_bytes` on this machine.
    /// Vulkan and CUDA copy weights off mmap, so they need roughly two copies.
    pub fn runtime_need_bytes(&self, file_bytes: u64) -> u64 {
        runtime_need_bytes_for(file_bytes, &self.accelerator)
    }
}

fn host_arch_labels() -> (String, String) {
    let process = std::env::consts::ARCH.to_string();
    let os = if super::gpu::windows_host_is_arm64() {
        "aarch64".into()
    } else {
        process.clone()
    };
    (process, os)
}

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * 1024 * 1024;

/// Estimated runtime need for a GGUF of `file_bytes` (weights + KV/overhead).
/// Metal, CPU, and OpenCL keep mmap. Vulkan and CUDA copy the weights.
pub fn runtime_need_bytes(file_bytes: u64) -> u64 {
    runtime_need_bytes_for(file_bytes, "")
}

fn runtime_need_bytes_for(file_bytes: u64, accelerator: &str) -> u64 {
    let copies = if matches!(accelerator, "Vulkan" | "CUDA") {
        2.0
    } else {
        1.15
    };
    (file_bytes as f64 * copies) as u64 + 2 * GIB
}

/// A curated, ordered catalog used for the first-run recommendation.
/// Rows are hardcoded, never fetched. They are general document models
/// (mixed-language shelves, chat, recipes). Coding checkpoints and
/// single-language specialists are not listed; Explore can still find them.
///
/// Order is capability, highest first (`CatalogStanding::sort_key`), then
/// smaller same-family fallbacks so lower RAM bands still have a pick.
/// The default install is the first row that fits RAM.
///
/// Missing an Artificial Analysis Intelligence Index is not a skip. A
/// new model may sit above scored rows when published benches show a
/// clear lead over the current pick for the bands it would take; mark
/// that `CatalogStanding::BenchLead`. The app never fetches AA or
/// leaderboards — this standing is chosen on the dev machine.
pub struct CatalogEntry {
    pub name: &'static str,
    pub family: &'static str,
    pub provider: &'static str,
    pub standing: CatalogStanding,
    pub hf_repo: &'static str,
    pub approx_bytes: u64,
    pub license: &'static str,
    pub released: &'static str,
    pub blurb: &'static str,
}

/// Why a catalog row sits where it does. Higher [`sort_key`](Self::sort_key)
/// is stronger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogStanding {
    /// Artificial Analysis Intelligence Index.
    Scored(u8),
    /// No Intelligence Index yet. Benches beat the scored document pick
    /// whose index is `above`; the row sorts just above that score.
    BenchLead { above: u8 },
}

impl CatalogStanding {
    /// Family heads in `CATALOG` must appear in this order, descending.
    pub const fn sort_key(self) -> u16 {
        match self {
            Self::Scored(score) => (score as u16) * 2,
            Self::BenchLead { above } => (above as u16) * 2 + 1,
        }
    }
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "Qwen3.8 27B",
        family: "Qwen",
        provider: "Alibaba",
        standing: CatalogStanding::Scored(52),
        hf_repo: "unsloth/Qwen3.8-27B-GGUF",
        approx_bytes: 16314 * MIB,
        license: "Apache-2.0",
        released: "2026-08",
        blurb: "Many languages. Strong on documents and everyday writing.",
    },
    CatalogEntry {
        name: "Muse Glimmer",
        family: "Muse",
        provider: "Meta",
        standing: CatalogStanding::Scored(35),
        hf_repo: "unsloth/Muse-Glimmer-30B-GGUF",
        approx_bytes: 15143 * MIB,
        license: "Apache-2.0",
        released: "2026-08",
        blurb: "From Meta. Needs a computer with plenty of memory.",
    },
    CatalogEntry {
        name: "Gemma 4 31B",
        family: "Gemma",
        provider: "Google",
        standing: CatalogStanding::Scored(30),
        hf_repo: "unsloth/gemma-4-31B-it-GGUF",
        approx_bytes: 17475 * MIB,
        license: "Apache-2.0",
        released: "2026-04",
        blurb: "Larger Gemma from Google. Big download.",
    },
    CatalogEntry {
        name: "Ornith-1.5 9B",
        family: "Ornith",
        provider: "DeepReinforce",
        standing: CatalogStanding::BenchLead { above: 22 },
        hf_repo: "ornith-ai/Ornith-1.5-9B-GGUF",
        approx_bytes: 5512 * MIB,
        license: "MIT",
        released: "2026-08",
        blurb: "Many languages. Documents and everyday writing on computers with 16 GB of memory.",
    },
    CatalogEntry {
        name: "Gemma 4 12B",
        family: "Gemma",
        provider: "Google",
        standing: CatalogStanding::Scored(22),
        hf_repo: "unsloth/gemma-4-12b-it-GGUF",
        approx_bytes: 6792 * MIB,
        license: "Apache-2.0",
        released: "2026-05",
        blurb: "From Google. Documents and chat.",
    },
    CatalogEntry {
        name: "Qwen3.5 4B",
        family: "Qwen",
        provider: "Alibaba",
        standing: CatalogStanding::Scored(20),
        hf_repo: "unsloth/Qwen3.5-4B-GGUF",
        approx_bytes: 2614 * MIB,
        license: "Apache-2.0",
        released: "2026-03",
        blurb: "Many languages. Documents and everyday writing on computers with 8 GB of memory.",
    },
    CatalogEntry {
        name: "gpt-oss 20B",
        family: "GPT",
        provider: "OpenAI",
        standing: CatalogStanding::Scored(15),
        hf_repo: "unsloth/gpt-oss-20b-GGUF",
        approx_bytes: 11086 * MIB,
        license: "Apache-2.0",
        released: "2025-08",
        blurb: "Open weights from OpenAI.",
    },
    CatalogEntry {
        name: "Gemma 4 E4B",
        family: "Gemma",
        provider: "Google",
        standing: CatalogStanding::Scored(12),
        hf_repo: "unsloth/gemma-4-E4B-it-GGUF",
        approx_bytes: 4747 * MIB,
        license: "Apache-2.0",
        released: "2026-03",
        blurb: "From Google. A smaller Gemma for tighter memory.",
    },
    CatalogEntry {
        name: "Ministral 3 14B",
        family: "Mistral",
        provider: "Mistral",
        standing: CatalogStanding::Scored(11),
        hf_repo: "unsloth/Ministral-3-14B-Instruct-2512-GGUF",
        approx_bytes: 7857 * MIB,
        license: "Apache-2.0",
        released: "2025-12",
        blurb: "Comfortable with long documents.",
    },
    CatalogEntry {
        name: "Ministral 3 8B",
        family: "Mistral",
        provider: "Mistral",
        standing: CatalogStanding::Scored(9),
        hf_repo: "unsloth/Ministral-3-8B-Instruct-2512-GGUF",
        approx_bytes: 4958 * MIB,
        license: "Apache-2.0",
        released: "2025-12",
        blurb: "Quicker replies.",
    },
    CatalogEntry {
        name: "Ministral 3 3B",
        family: "Mistral",
        provider: "Mistral",
        standing: CatalogStanding::Scored(7),
        hf_repo: "unsloth/Ministral-3-3B-Instruct-2512-GGUF",
        approx_bytes: 2047 * MIB,
        license: "Apache-2.0",
        released: "2025-12",
        blurb: "Fits when memory is tight.",
    },
    CatalogEntry {
        name: "Qwen3.5 2B",
        family: "Qwen",
        provider: "Alibaba",
        standing: CatalogStanding::Scored(7),
        hf_repo: "unsloth/Qwen3.5-2B-GGUF",
        approx_bytes: 1222 * MIB,
        license: "Apache-2.0",
        released: "2026-01",
        blurb: "For trying Larra without waiting.",
    },
    CatalogEntry {
        name: "Phi-4 Mini",
        family: "Phi",
        provider: "Microsoft",
        standing: CatalogStanding::Scored(6),
        hf_repo: "unsloth/Phi-4-mini-instruct-GGUF",
        approx_bytes: 2376 * MIB,
        license: "MIT",
        released: "2024-12",
        blurb: "From Microsoft. Careful answers in a small file.",
    },
    CatalogEntry {
        name: "Granite 4.1 3B",
        family: "Granite",
        provider: "IBM",
        standing: CatalogStanding::Scored(4),
        hf_repo: "unsloth/granite-4.1-3b-GGUF",
        approx_bytes: 2002 * MIB,
        license: "Apache-2.0",
        released: "2026-04",
        blurb: "Small file, short answers.",
    },
    CatalogEntry {
        name: "Gemma 3 1B",
        family: "Gemma",
        provider: "Google",
        standing: CatalogStanding::Scored(1),
        hf_repo: "unsloth/gemma-3-1b-it-GGUF",
        approx_bytes: 769 * MIB,
        license: "Gemma",
        released: "2025-03",
        blurb: "Smallest pick. Installs in a minute.",
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub name: String,
    pub reference: String,
    pub provider: String,
    pub approx_bytes: u64,
    pub license: String,
    pub released: String,
    pub blurb: String,
}

fn to_recommendation(entry: &CatalogEntry) -> Recommendation {
    Recommendation {
        name: entry.name.to_string(),
        reference: entry.hf_repo.to_string(),
        provider: entry.provider.to_string(),
        approx_bytes: entry.approx_bytes,
        license: entry.license.to_string(),
        released: entry.released.to_string(),
        blurb: localized_blurb(entry),
    }
}

fn blurb_slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn localized_blurb(entry: &CatalogEntry) -> String {
    let key = format!("catalog.blurbs.{}", blurb_slug(entry.name));
    let text = rust_i18n::t!(&key).to_string();
    if text.is_empty() || text == key {
        entry.blurb.to_string()
    } else {
        text
    }
}

fn fits(entry: &CatalogEntry, profile: &MachineProfile) -> bool {
    profile.runtime_need_bytes(entry.approx_bytes) <= profile.model_budget_bytes()
}

fn forced_recommend_index() -> Option<usize> {
    let name = std::env::var("REBOST_FORCE_RECOMMEND").ok()?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    CATALOG
        .iter()
        .position(|entry| entry.name.eq_ignore_ascii_case(name))
}

fn pick_index(profile: &MachineProfile) -> usize {
    if let Some(index) = forced_recommend_index() {
        return index;
    }
    CATALOG
        .iter()
        .position(|entry| fits(entry, profile))
        .unwrap_or(CATALOG.len() - 1)
}

/// Recommended model: the first catalog row that fits in RAM
/// with headroom. Catalog order is capability (`CatalogStanding`), not a
/// live leaderboard fetch.
pub fn recommend(profile: &MachineProfile) -> Recommendation {
    to_recommendation(&CATALOG[pick_index(profile)])
}

/// Other-family document rows that fit and are a smaller download
/// than the primary. One sibling per family (the largest that still fits).
fn alt_candidates<'a>(
    profile: &'a MachineProfile,
    primary: &'a CatalogEntry,
) -> Vec<&'a CatalogEntry> {
    let mut by_family: Vec<&CatalogEntry> = Vec::new();
    for entry in CATALOG {
        if entry.name == primary.name || entry.family == primary.family {
            continue;
        }
        if !fits(entry, profile) {
            continue;
        }
        if entry.approx_bytes >= primary.approx_bytes {
            continue;
        }
        match by_family
            .iter_mut()
            .find(|picked| picked.family == entry.family)
        {
            Some(existing) => {
                if entry.approx_bytes > existing.approx_bytes {
                    *existing = entry;
                }
            }
            None => by_family.push(entry),
        }
    }
    by_family
}

/// Up to `n` smaller catalog models that also fit, each a different family.
/// First is the strongest *step-down* (file ≤80% of the primary) so a
/// near-twin like Muse does not beat Ornith next to Qwen 27B. If nothing
/// is that much smaller, first is the strongest smaller row. Second is
/// the remaining row whose file is closest to the primary.
pub fn smaller_alternatives(profile: &MachineProfile, n: usize) -> Vec<Recommendation> {
    if n == 0 {
        return Vec::new();
    }
    let primary = &CATALOG[pick_index(profile)];
    let candidates = alt_candidates(profile, primary);
    let step_max = primary.approx_bytes.saturating_mul(80) / 100;
    let step_downs: Vec<&CatalogEntry> = candidates
        .iter()
        .copied()
        .filter(|entry| entry.approx_bytes <= step_max)
        .collect();
    let capable_pool = if step_downs.is_empty() {
        candidates.as_slice()
    } else {
        step_downs.as_slice()
    };

    let mut chosen: Vec<&CatalogEntry> = Vec::new();
    if let Some(capable) = capable_pool
        .iter()
        .max_by(|a, b| {
            a.standing
                .sort_key()
                .cmp(&b.standing.sort_key())
                .then(a.approx_bytes.cmp(&b.approx_bytes))
        })
        .copied()
    {
        chosen.push(capable);
    }
    if n >= 2 && !chosen.is_empty() {
        if let Some(near) = candidates
            .iter()
            .filter(|entry| entry.name != chosen[0].name)
            .min_by(|a, b| {
                let da = a.approx_bytes.abs_diff(primary.approx_bytes);
                let db = b.approx_bytes.abs_diff(primary.approx_bytes);
                da.cmp(&db)
                    .then(b.standing.sort_key().cmp(&a.standing.sort_key()))
            })
            .copied()
        {
            chosen.push(near);
        }
    }
    chosen.truncate(n);
    chosen.into_iter().map(to_recommendation).collect()
}

/// Up to `n` catalog models that fit this machine and are not already
/// installed. Same order as first run: the default document pick, then
/// smaller alternatives from other families.
pub fn uninstalled_suggestions(
    profile: &MachineProfile,
    installed_reference: Option<&str>,
    n: usize,
) -> Vec<Recommendation> {
    if n == 0 {
        return Vec::new();
    }
    let skip = |reference: &str| {
        installed_reference.is_some_and(|installed| installed.eq_ignore_ascii_case(reference))
    };
    let mut out = Vec::new();
    let primary = recommend(profile);
    if fits(&CATALOG[pick_index(profile)], profile) && !skip(&primary.reference) {
        out.push(primary);
    }
    for alt in smaller_alternatives(profile, n) {
        if skip(&alt.reference) {
            continue;
        }
        if out.iter().any(|picked| picked.reference == alt.reference) {
            continue;
        }
        out.push(alt);
        if out.len() >= n {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_scales_with_ram() {
        let mk = |gb: u64| MachineProfile {
            total_ram_bytes: gb * GIB,
            cpu: "test".into(),
            accelerator: "Metal".into(),
            free_disk_bytes: 500 * GIB,
            process_arch: "test".into(),
            os_arch: "test".into(),
        };
        assert_eq!(recommend(&mk(48)).name, "Qwen3.8 27B");
        assert_eq!(recommend(&mk(32)).name, "Qwen3.8 27B");
        assert_eq!(recommend(&mk(24)).name, "Ornith-1.5 9B");
        assert_eq!(recommend(&mk(16)).name, "Ornith-1.5 9B");
        assert_eq!(recommend(&mk(8)).name, "Qwen3.5 4B");

        let alts48: Vec<_> = smaller_alternatives(&mk(48), 2)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(alts48, ["Ornith-1.5 9B", "Muse Glimmer"]);
        let alts24: Vec<_> = smaller_alternatives(&mk(24), 2)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(alts24, ["Qwen3.5 4B", "Ministral 3 8B"]);
        assert_eq!(
            smaller_alternatives(&mk(16), 2)
                .into_iter()
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            ["Qwen3.5 4B", "Ministral 3 8B"]
        );
        assert_eq!(
            smaller_alternatives(&mk(8), 2)
                .into_iter()
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            ["Ministral 3 3B", "Phi-4 Mini"]
        );
        assert!(smaller_alternatives(&mk(4), 2).is_empty());
    }

    #[test]
    fn family_heads_descend_by_standing() {
        let mut seen = std::collections::HashSet::new();
        let mut keys = Vec::new();
        for entry in CATALOG {
            if seen.insert(entry.family) {
                keys.push((entry.family, entry.standing.sort_key()));
            }
        }
        let mut sorted = keys.clone();
        sorted.sort_by_key(|(_, key)| std::cmp::Reverse(*key));
        assert_eq!(
            keys, sorted,
            "first row of each family must be capability order"
        );
    }

    #[test]
    fn bench_lead_sorts_just_above_the_score_it_beats() {
        assert!(
            CatalogStanding::BenchLead { above: 22 }.sort_key()
                > CatalogStanding::Scored(22).sort_key()
        );
        assert!(
            CatalogStanding::BenchLead { above: 22 }.sort_key()
                < CatalogStanding::Scored(23).sort_key()
        );
    }

    #[test]
    fn ornith_sits_above_gemma_12b_e4b_and_gpt_oss() {
        let standing = |name: &str| {
            CATALOG
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.standing.sort_key())
                .expect(name)
        };
        let ornith = standing("Ornith-1.5 9B");
        assert!(ornith > standing("Gemma 4 12B"));
        assert!(ornith > standing("Gemma 4 E4B"));
        assert!(ornith > standing("gpt-oss 20B"));
        assert!(standing("Qwen3.5 4B") < standing("Gemma 4 12B"));
        assert!(standing("Qwen3.5 4B") > standing("gpt-oss 20B"));
        assert_eq!(
            CATALOG
                .iter()
                .position(|entry| entry.name == "Ornith-1.5 9B"),
            CATALOG
                .iter()
                .position(|entry| entry.name == "Gemma 4 31B")
                .map(|i| i + 1)
        );
    }

    #[test]
    fn uninstalled_suggestions_skip_the_active_model() {
        let mk = |gb: u64| MachineProfile {
            total_ram_bytes: gb * GIB,
            cpu: "test".into(),
            accelerator: "Metal".into(),
            free_disk_bytes: 500 * GIB,
            process_arch: "test".into(),
            os_arch: "test".into(),
        };
        let none: Vec<_> = uninstalled_suggestions(&mk(48), None, 2)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(none, ["Qwen3.8 27B", "Ornith-1.5 9B"]);

        let installed = recommend(&mk(48)).reference;
        let skipped: Vec<_> = uninstalled_suggestions(&mk(48), Some(&installed), 2)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(skipped, ["Ornith-1.5 9B", "Muse Glimmer"]);
        assert!(uninstalled_suggestions(&mk(4), None, 2).is_empty());
    }

    #[test]
    fn gemma_license_strings_match_upstream_policy() {
        // Gemma 4 is Apache-2.0; Gemma 3 (and earlier) stay on Gemma Terms.
        // https://opensource.googleblog.com/2026/03/gemma-4-expanding-the-gemmaverse-with-apache-20.html
        for entry in CATALOG {
            if entry.name.starts_with("Gemma 4") {
                assert_eq!(entry.license, "Apache-2.0", "{}", entry.name);
            }
            if entry.name.starts_with("Gemma 3") {
                assert_eq!(entry.license, "Gemma", "{}", entry.name);
            }
        }
    }

    #[test]
    fn vulkan_counts_two_copies_of_the_weights() {
        let mk = |accel: &str| MachineProfile {
            total_ram_bytes: 16 * GIB,
            cpu: "test".into(),
            accelerator: accel.into(),
            free_disk_bytes: 500 * GIB,
            process_arch: "test".into(),
            os_arch: "test".into(),
        };
        let file = 5 * GIB;
        let metal = mk("Metal");
        let vulkan = mk("Vulkan");
        assert_eq!(vulkan.runtime_need_bytes(file), 12 * GIB);
        assert!(metal.runtime_need_bytes(file) < vulkan.runtime_need_bytes(file));
        assert!(metal.runtime_need_bytes(file) <= metal.model_budget_bytes());
        assert!(vulkan.runtime_need_bytes(file) > vulkan.model_budget_bytes());
    }

    #[test]
    fn blurb_slug_matches_catalog_keys() {
        assert_eq!(blurb_slug("Qwen3.8 27B"), "qwen3_8_27b");
        assert_eq!(blurb_slug("gpt-oss 20B"), "gpt_oss_20b");
        assert_eq!(blurb_slug("Gemma 4 E4B"), "gemma_4_e4b");
        assert_eq!(blurb_slug("Muse Glimmer"), "muse_glimmer");
        assert_eq!(blurb_slug("Phi-4 Mini"), "phi_4_mini");
        assert_eq!(blurb_slug("Granite 4.1 3B"), "granite_4_1_3b");
    }

    #[test]
    fn english_blurb_matches_catalog_source() {
        let rec = to_recommendation(&CATALOG[0]);
        assert_eq!(rec.blurb, CATALOG[0].blurb);
    }
}
