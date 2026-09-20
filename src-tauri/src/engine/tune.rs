//! llama-server flags that follow the machine, the loaded AI, and KV room.

use std::path::{Path, PathBuf};

use super::catalog::MachineProfile;
use super::gguf;
use super::pin::EnginePin;
use crate::settings::ActiveModel;

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;
const CHARS_PER_TOKEN: f64 = 3.6;
/// Recent-history cap used when the window still has room.
const HISTORY_OVERHEAD_CHARS: usize = 6_000;
/// Standing system prompt (boilerplate, shelf inventory). House rules and
/// the current user text are counted separately in [`prompt_budget`].
const SYSTEM_BASE_CHARS: usize = 2_500;
const CTX_PADDING_TOKENS: u32 = 512;
/// KV plus OS headroom after the weight file is mapped.
const LEFTOVER_FOR_WIDE_CTX: u64 = (5 * GIB) / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelClass {
    Tiny,
    Small,
    Mid,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelFamily {
    Qwen,
    Mistral,
    Gemma,
    Granite,
    Gpt,
    Muse,
    Phi,
    Other,
}

/// Enough of the loaded AI to pick `-c` and the answer cap.
#[derive(Debug, Clone)]
pub struct ModelHint {
    pub name: String,
    pub file: String,
    pub file_bytes: u64,
    pub gguf_path: Option<PathBuf>,
}

impl ModelHint {
    pub fn from_active(model: &ActiveModel, models_dir: &Path) -> Self {
        let path = models_dir.join(&model.file);
        let file_bytes = if model.size_bytes > 0 {
            model.size_bytes
        } else {
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
        };
        Self {
            name: model.name.clone(),
            file: model.file.clone(),
            file_bytes,
            gguf_path: Some(path),
        }
    }
}

/// Answer length cap. Short replies still stop at the model's end token.
/// Sized so a long file keeps more of the window than the essay.
pub fn max_answer_tokens(context_tokens: u32) -> u32 {
    answer_tokens_for(context_tokens, None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPlan {
    pub context_tokens: u32,
    pub answer_tokens: u32,
    pub batch: u32,
    pub ubatch: u32,
    pub no_mmap: bool,
    pub flash_attn: &'static str,
    /// llama.cpp `--cache-type-k/v`. OpenCL Adreno crashes on q8_0 KV.
    pub cache_type: &'static str,
    pub gpu_layers: u32,
}

impl SpawnPlan {
    #[cfg(test)]
    pub fn from_profile(profile: &MachineProfile, pin: &EnginePin) -> Self {
        Self::for_model(profile, pin, None)
    }

    pub fn for_model(profile: &MachineProfile, pin: &EnginePin, model: Option<&ModelHint>) -> Self {
        let (batch, ubatch) = batch_for(profile, pin);
        let class = model.map(classify_hint);
        let context_tokens = context_tokens_for(profile, pin, model, class);
        Self {
            context_tokens,
            answer_tokens: answer_tokens_for(context_tokens, class),
            batch,
            ubatch,
            no_mmap: super::pin::no_mmap_for(pin),
            flash_attn: flash_attn_for(pin),
            cache_type: cache_type_for(pin),
            gpu_layers: gpu_layers_for(pin),
        }
    }
}

fn flash_attn_for(pin: &EnginePin) -> &'static str {
    match pin.accelerator {
        "CPU" | "OpenCL" => "auto",
        _ => "on",
    }
}

fn cache_type_for(pin: &EnginePin) -> &'static str {
    match pin.accelerator {
        "OpenCL" => "f16",
        _ => "q8_0",
    }
}

fn gpu_layers_for(pin: &EnginePin) -> u32 {
    match pin.accelerator {
        "CPU" => 0,
        "Vulkan" if super::gpu::windows_host_is_arm64() => 0,
        _ => 99,
    }
}

/// First-token wait. CPU (and the x64-on-ARM Vulkan copy running on CPU)
/// can spend a minute on prefill before any SSE bytes arrive.
pub fn chat_stall_timeout(plan: &SpawnPlan) -> std::time::Duration {
    if plan.gpu_layers == 0 || plan.flash_attn == "auto" {
        std::time::Duration::from_secs(180)
    } else {
        std::time::Duration::from_secs(90)
    }
}

fn context_tokens_for(
    profile: &MachineProfile,
    pin: &EnginePin,
    model: Option<&ModelHint>,
    class: Option<ModelClass>,
) -> u32 {
    let machine = machine_max_ctx(profile, pin, model.map(|m| m.file_bytes));
    let Some(hint) = model else {
        return machine_base_ctx(profile, pin).min(machine);
    };
    let family = infer_family(&hint.name, &hint.file);
    let class = class.unwrap_or(ModelClass::Mid);
    let native = native_ctx(hint, family, class);
    let target = class_target_ctx(class, family, profile.total_ram_bytes, pin);
    let want = target.min(native).min(machine);
    // Chat's standing prompt does not fit a 2k window. Stay at 4k when the
    // machine can, even if the file's trained length is smaller.
    want.max(4096.min(machine))
}

fn machine_base_ctx(profile: &MachineProfile, pin: &EnginePin) -> u32 {
    let ram = profile.total_ram_bytes;
    match pin.accelerator {
        "Metal" => 8192,
        "CUDA" | "Vulkan" => {
            if ram >= 16 * GIB {
                8192
            } else {
                4096
            }
        }
        "OpenCL" => {
            if ram >= 16 * GIB {
                6144
            } else {
                4096
            }
        }
        _ => {
            if ram >= 16 * GIB {
                6144
            } else {
                4096
            }
        }
    }
}

fn machine_max_ctx(profile: &MachineProfile, pin: &EnginePin, file_bytes: Option<u64>) -> u32 {
    let ram = profile.total_ram_bytes;
    let unified = pin.accelerator == "Metal";
    // Weights share system memory only on unified memory. Discrete VRAM is
    // not measured here, so leftover RAM must not unlock a wide KV cache.
    if unified {
        if let Some(bytes) = file_bytes.filter(|n| *n > 0) {
            let leftover = ram.saturating_sub(bytes);
            if leftover < LEFTOVER_FOR_WIDE_CTX {
                return 4096;
            }
        }
    }
    if ram < 16 * GIB {
        return 4096;
    }
    if !is_gpu(pin) {
        return machine_base_ctx(profile, pin);
    }
    if unified && ram >= 24 * GIB {
        return 16384;
    }
    8192
}

fn is_gpu(pin: &EnginePin) -> bool {
    matches!(pin.accelerator, "Metal" | "CUDA" | "Vulkan")
}

fn class_target_ctx(class: ModelClass, family: ModelFamily, ram: u64, pin: &EnginePin) -> u32 {
    let gpu = is_gpu(pin);
    let unified = pin.accelerator == "Metal";
    match class {
        ModelClass::Tiny => {
            if gpu
                && matches!(
                    family,
                    ModelFamily::Qwen | ModelFamily::Mistral | ModelFamily::Granite
                )
            {
                8192
            } else {
                4096
            }
        }
        ModelClass::Small => {
            // Gemma E4B has the window on paper and loses the thread in practice.
            if family == ModelFamily::Gemma {
                8192
            } else if unified && ram >= 24 * GIB {
                16384
            } else {
                8192
            }
        }
        ModelClass::Mid => {
            if unified && ram >= 32 * GIB {
                16384
            } else {
                8192
            }
        }
        ModelClass::Large => {
            if unified && ram >= 48 * GIB {
                12288
            } else {
                8192
            }
        }
    }
}

fn native_ctx(hint: &ModelHint, family: ModelFamily, class: ModelClass) -> u32 {
    let from_file = hint
        .gguf_path
        .as_deref()
        .and_then(gguf::read_context_length)
        .filter(|n| *n >= 2048);
    let typical = match (family, class) {
        (ModelFamily::Gemma, ModelClass::Tiny) => 8192,
        (
            ModelFamily::Qwen
            | ModelFamily::Mistral
            | ModelFamily::Gpt
            | ModelFamily::Granite
            | ModelFamily::Muse
            | ModelFamily::Gemma
            | ModelFamily::Phi,
            _,
        ) => 32768,
        (ModelFamily::Other, _) => 8192,
    };
    from_file.unwrap_or(typical)
}

fn answer_tokens_for(context_tokens: u32, class: Option<ModelClass>) -> u32 {
    let by_class = match class {
        Some(ModelClass::Tiny) => 768,
        Some(ModelClass::Small) => 1536,
        Some(ModelClass::Mid) | Some(ModelClass::Large) => 2048,
        None if context_tokens >= 8192 => 2048,
        None if context_tokens >= 6144 => 1536,
        None => 768,
    };
    let by_window = if context_tokens <= 4096 {
        768
    } else if context_tokens < 8192 {
        1536
    } else {
        2048
    };
    by_class.min(by_window).min(context_tokens / 4).max(512)
}

fn classify_hint(hint: &ModelHint) -> ModelClass {
    classify_model(&hint.name, &hint.file, hint.file_bytes)
}

fn classify_model(name: &str, file: &str, file_bytes: u64) -> ModelClass {
    if let Some(b) = param_billions(name).or_else(|| param_billions(file)) {
        return class_from_params(b);
    }
    class_from_file_bytes(file_bytes)
}

fn class_from_params(billions: f32) -> ModelClass {
    if billions < 4.0 {
        ModelClass::Tiny
    } else if billions < 10.0 {
        ModelClass::Small
    } else if billions < 24.0 {
        ModelClass::Mid
    } else {
        ModelClass::Large
    }
}

fn class_from_file_bytes(bytes: u64) -> ModelClass {
    if bytes == 0 || bytes < 2500 * MIB {
        ModelClass::Tiny
    } else if bytes < 6000 * MIB {
        ModelClass::Small
    } else if bytes < 13000 * MIB {
        ModelClass::Mid
    } else {
        ModelClass::Large
    }
}

fn param_billions(text: &str) -> Option<f32> {
    let lower = text.to_ascii_lowercase();
    if lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == "e4b")
    {
        return Some(4.0);
    }
    if lower.contains("phi-4-mini") || lower.contains("phi4-mini") || lower.contains("phi4_mini") {
        return Some(4.0);
    }
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'b' {
                let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
                let after_ok = i + 1 == bytes.len() || !bytes[i + 1].is_ascii_alphanumeric();
                if before_ok && after_ok {
                    if let Ok(n) = lower[start..i].parse::<f32>() {
                        if (1.0..256.0).contains(&n) {
                            return Some(n);
                        }
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn infer_family(name: &str, file: &str) -> ModelFamily {
    let hay = format!("{name} {file}").to_ascii_lowercase();
    if hay.contains("qwen") || hay.contains("ornith") {
        ModelFamily::Qwen
    } else if hay.contains("ministral") || hay.contains("mistral") {
        ModelFamily::Mistral
    } else if hay.contains("gemma") {
        ModelFamily::Gemma
    } else if hay.contains("granite") {
        ModelFamily::Granite
    } else if hay.contains("gpt-oss") || hay.contains("gpt_oss") || hay.contains("gptoss") {
        ModelFamily::Gpt
    } else if hay.contains("muse") || hay.contains("glimmer") {
        ModelFamily::Muse
    } else if hay.contains("phi") {
        ModelFamily::Phi
    } else {
        ModelFamily::Other
    }
}

fn batch_for(profile: &MachineProfile, pin: &EnginePin) -> (u32, u32) {
    let ram = profile.total_ram_bytes;
    match pin.accelerator {
        "Metal" => {
            if ram >= 48 * GIB {
                (2048, 2048)
            } else if ram >= 24 * GIB {
                (2048, 1024)
            } else {
                (1024, 512)
            }
        }
        "CUDA" | "Vulkan" => {
            if ram >= 16 * GIB {
                (2048, 512)
            } else {
                (1024, 256)
            }
        }
        "OpenCL" => {
            if ram >= 16 * GIB {
                (1024, 512)
            } else {
                (512, 256)
            }
        }
        _ => {
            if ram >= 24 * GIB {
                (512, 512)
            } else {
                (256, 256)
            }
        }
    }
}

/// Characters left for the prompt after the answer slot and padding.
pub fn prompt_room_chars(context_tokens: u32, answer_tokens: u32) -> usize {
    let prompt_tokens = context_tokens.saturating_sub(answer_tokens + CTX_PADDING_TOKENS);
    (prompt_tokens as f64 * CHARS_PER_TOKEN) as usize
}

/// How a turn spends the prompt window: user first, then history, then files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptBudget {
    pub user_chars: usize,
    pub history_chars: usize,
    pub retrieval_chars: usize,
}

/// Fit house rules, the current question, recent turns, and retrieval into `-c`.
pub fn prompt_budget(
    context_tokens: u32,
    answer_tokens: u32,
    requested_retrieval: usize,
    house_rules_chars: usize,
    user_chars: usize,
) -> PromptBudget {
    let mut left = prompt_room_chars(context_tokens, answer_tokens)
        .saturating_sub(SYSTEM_BASE_CHARS.saturating_add(house_rules_chars));
    let user = user_chars.min(left);
    left = left.saturating_sub(user);
    let history = HISTORY_OVERHEAD_CHARS.min(left);
    left = left.saturating_sub(history);
    PromptBudget {
        user_chars: user,
        history_chars: history,
        retrieval_chars: requested_retrieval.min(left),
    }
}

/// Cap retrieval so the prompt plus the answer still fit in `-c`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn retrieval_char_budget_with(
    context_tokens: u32,
    answer_tokens: u32,
    requested: usize,
) -> usize {
    prompt_budget(context_tokens, answer_tokens, requested, 0, 0).retrieval_chars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pin::pin_for;

    fn profile(gb: u64, accel: &str) -> MachineProfile {
        MachineProfile {
            total_ram_bytes: gb * GIB,
            cpu: "test".into(),
            accelerator: accel.into(),
            free_disk_bytes: 200 * GIB,
            process_arch: "test".into(),
            os_arch: "test".into(),
        }
    }

    fn hint(name: &str, file: &str, mib: u64) -> ModelHint {
        ModelHint {
            name: name.into(),
            file: file.into(),
            file_bytes: mib * MIB,
            gguf_path: None,
        }
    }

    fn write_gguf_ctx(dir: &std::path::Path, n: u32) -> PathBuf {
        let path = dir.join("m.gguf");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        let key = b"llama.context_length";
        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&n.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn m_series_max_gets_large_batches_and_8k() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let plan = SpawnPlan::from_profile(&profile(128, "Metal"), pin);
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!(plan.answer_tokens, 2048);
        assert_eq!((plan.batch, plan.ubatch), (2048, 2048));
        assert!(!plan.no_mmap);
        assert_eq!(plan.flash_attn, "on");
    }

    #[test]
    fn sixteen_gb_mac_keeps_modest_ubatch() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let plan = SpawnPlan::from_profile(&profile(16, "Metal"), pin);
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!((plan.batch, plan.ubatch), (1024, 512));
    }

    #[test]
    fn vulkan_windows_uses_no_mmap_and_q8_sized_ctx() {
        let pin = pin_for("windows", "x86_64").unwrap();
        let plan = SpawnPlan::from_profile(&profile(32, "Vulkan"), pin);
        assert!(plan.no_mmap);
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!((plan.batch, plan.ubatch), (2048, 512));
        assert_eq!(plan.flash_attn, "on");
    }

    #[test]
    fn arm_cpu_stays_small() {
        let pin = pin_for("windows", "aarch64").unwrap();
        let plan = SpawnPlan::from_profile(&profile(16, "CPU"), pin);
        assert_eq!(plan.context_tokens, 6144);
        assert_eq!((plan.batch, plan.ubatch), (256, 256));
        assert!(!plan.no_mmap);
        assert_eq!(plan.flash_attn, "auto");
        assert_eq!(plan.cache_type, "q8_0");
        assert_eq!(plan.gpu_layers, 0);
    }

    #[test]
    fn cpu_and_opencl_wait_longer_for_the_first_token() {
        let cpu = pin_for("windows", "aarch64").unwrap();
        let cpu_plan = SpawnPlan::from_profile(&profile(16, "CPU"), cpu);
        assert_eq!(
            chat_stall_timeout(&cpu_plan),
            std::time::Duration::from_secs(180)
        );
        let vulkan = pin_for("windows", "x86_64").unwrap();
        let vulkan_plan = SpawnPlan::from_profile(&profile(16, "Vulkan"), vulkan);
        assert_eq!(
            chat_stall_timeout(&vulkan_plan),
            std::time::Duration::from_secs(90)
        );
    }

    #[test]
    fn retrieval_budget_fits_inside_context() {
        let room = retrieval_char_budget_with(4096, max_answer_tokens(4096), 26_000);
        assert!(room < 26_000);
        assert!(room > 0);
        assert!(retrieval_char_budget_with(4096, max_answer_tokens(4096), 9_000) < 9_000);
        let wide = retrieval_char_budget_with(8192, max_answer_tokens(8192), 9_000);
        assert_eq!(wide, 9_000);
    }

    #[test]
    fn answer_cap_follows_class_and_leaves_file_room() {
        assert_eq!(max_answer_tokens(4096), 768);
        assert_eq!(max_answer_tokens(6144), 1536);
        assert_eq!(max_answer_tokens(8192), 2048);
        assert_eq!(answer_tokens_for(8192, Some(ModelClass::Small)), 1536);
        assert_eq!(answer_tokens_for(4096, Some(ModelClass::Tiny)), 768);
    }

    #[test]
    fn adreno_opencl_is_gentler_than_metal() {
        let pin = crate::engine::pin::optional_pin_for("windows", "aarch64", "OpenCL").unwrap();
        let plan = SpawnPlan::from_profile(&profile(16, "OpenCL"), pin);
        assert_eq!(plan.context_tokens, 6144);
        assert_eq!((plan.batch, plan.ubatch), (1024, 512));
        assert!(!plan.no_mmap);
        assert_eq!(plan.flash_attn, "auto");
    }

    #[test]
    fn adreno_opencl_uses_f16_kv_cache() {
        let pin = crate::engine::pin::optional_pin_for("windows", "aarch64", "OpenCL").unwrap();
        let plan = SpawnPlan::from_profile(&profile(16, "OpenCL"), pin);
        assert_eq!(plan.cache_type, "f16");
        assert_eq!(plan.gpu_layers, 99);
        assert_eq!(plan.flash_attn, "auto");
    }

    #[test]
    fn eight_gb_vulkan_drops_to_4k() {
        let pin = pin_for("windows", "x86_64").unwrap();
        let plan = SpawnPlan::from_profile(&profile(8, "Vulkan"), pin);
        assert_eq!(plan.context_tokens, 4096);
        assert_eq!((plan.batch, plan.ubatch), (1024, 256));
    }

    #[test]
    fn classifies_catalog_names() {
        assert_eq!(
            classify_model("Gemma 3 1B", "gemma-3-1b-it-Q4_K_M.gguf", 769 * MIB),
            ModelClass::Tiny
        );
        assert_eq!(
            classify_model("Gemma 4 E4B", "gemma-4-E4B-it.gguf", 4747 * MIB),
            ModelClass::Small
        );
        assert_eq!(
            classify_model("Ministral 3 14B", "Ministral-3-14B.gguf", 7857 * MIB),
            ModelClass::Mid
        );
        assert_eq!(
            classify_model("Qwen3.8 27B", "Qwen3.8-27B.gguf", 16314 * MIB),
            ModelClass::Large
        );
        assert_eq!(
            classify_model("gpt-oss 20B", "gpt-oss-20b.gguf", 11086 * MIB),
            ModelClass::Mid
        );
        assert_eq!(
            classify_model("Ornith-1.5 9B", "Ornith-1.5-9B-Q4_K_M.gguf", 5512 * MIB),
            ModelClass::Small
        );
        assert_eq!(
            classify_model("Qwen3.5 4B", "Qwen3.5-4B-Q4_K_M.gguf", 2614 * MIB),
            ModelClass::Small
        );
        assert_eq!(
            classify_model("Phi-4 Mini", "Phi-4-mini-instruct-Q4_K_M.gguf", 2376 * MIB),
            ModelClass::Small
        );
    }

    #[test]
    fn gemma_1b_stays_on_4k() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Gemma 3 1B", "gemma-3-1b.gguf", 769);
        let plan = SpawnPlan::for_model(&profile(16, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 4096);
        assert_eq!(plan.answer_tokens, 768);
    }

    #[test]
    fn granite_tiny_may_use_8k_on_metal() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Granite 4.1 3B", "granite-4.1-3b.gguf", 2002);
        let plan = SpawnPlan::for_model(&profile(16, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!(plan.answer_tokens, 768);
    }

    #[test]
    fn ministral_14b_stretches_on_32gb_metal() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Ministral 3 14B", "Ministral-3-14B.gguf", 7857);
        let plan = SpawnPlan::for_model(&profile(32, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 16384);
        assert_eq!(plan.answer_tokens, 2048);
    }

    #[test]
    fn large_qwen_drops_to_4k_when_weights_fill_ram() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Qwen3.8 27B", "Qwen3.8-27B.gguf", 16314);
        let plan = SpawnPlan::for_model(&profile(16, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 4096);
        assert_eq!(plan.answer_tokens, 768);
    }

    #[test]
    fn large_qwen_stretches_only_on_48gb_metal() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Qwen3.8 27B", "Qwen3.8-27B.gguf", 16314);
        let plan = SpawnPlan::for_model(&profile(48, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 12288);
        assert_eq!(plan.answer_tokens, 2048);
    }

    #[test]
    fn gemma_e4b_does_not_stretch() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Gemma 4 E4B", "gemma-4-E4B-it.gguf", 4747);
        let plan = SpawnPlan::for_model(&profile(32, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!(plan.answer_tokens, 1536);
    }

    #[test]
    fn small_mistral_stretches_on_24gb_metal() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Ministral 3 8B", "Ministral-3-8B.gguf", 5114);
        let plan = SpawnPlan::for_model(&profile(24, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 16384);
        assert_eq!(plan.answer_tokens, 1536);
    }

    #[test]
    fn mid_stays_8k_on_24gb() {
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = hint("Ministral 3 14B", "Ministral-3-14B.gguf", 7857);
        let plan = SpawnPlan::for_model(&profile(24, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!(plan.answer_tokens, 2048);
    }

    #[test]
    fn large_does_not_stretch_on_vulkan() {
        let pin = pin_for("windows", "x86_64").unwrap();
        let model = hint("Qwen3.8 27B", "Qwen3.8-27B.gguf", 16314);
        let plan = SpawnPlan::for_model(&profile(48, "Vulkan"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!(plan.answer_tokens, 2048);
    }

    #[test]
    fn eight_gb_gpu_caps_a_mid_model() {
        let pin = pin_for("windows", "x86_64").unwrap();
        let model = hint("Ministral 3 14B", "Ministral-3-14B.gguf", 7857);
        let plan = SpawnPlan::for_model(&profile(8, "Vulkan"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 4096);
        assert_eq!(plan.answer_tokens, 768);
    }

    #[test]
    fn trained_context_is_a_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_gguf_ctx(dir.path(), 4096);
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = ModelHint {
            name: "Ministral 3 8B".into(),
            file: "m.gguf".into(),
            file_bytes: 5114 * MIB,
            gguf_path: Some(path),
        };
        let plan = SpawnPlan::for_model(&profile(32, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 4096);
        assert_eq!(plan.answer_tokens, 768);
    }

    #[test]
    fn two_k_trained_context_still_gets_a_4k_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_gguf_ctx(dir.path(), 2048);
        let pin = pin_for("macos", "aarch64").unwrap();
        let model = ModelHint {
            name: "Ministral 3 8B".into(),
            file: "m.gguf".into(),
            file_bytes: 5114 * MIB,
            gguf_path: Some(path),
        };
        let plan = SpawnPlan::for_model(&profile(32, "Metal"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 4096);
        assert_eq!(plan.answer_tokens, 768);
        assert!(retrieval_char_budget_with(plan.context_tokens, plan.answer_tokens, 9_000) > 0);
        assert_eq!(retrieval_char_budget_with(2048, 512, 9_000), 0);
    }

    #[test]
    fn discrete_gpu_does_not_stretch_from_system_ram() {
        let pin = pin_for("windows", "x86_64").unwrap();
        let model = hint("Ministral 3 8B", "Ministral-3-8B.gguf", 5114);
        let plan = SpawnPlan::for_model(&profile(32, "Vulkan"), pin, Some(&model));
        assert_eq!(plan.context_tokens, 8192);
        assert_eq!(plan.answer_tokens, 1536);
    }

    #[test]
    fn small_class_leaves_more_file_room_than_mid() {
        let small = retrieval_char_budget_with(8192, 1536, 26_000);
        let mid = retrieval_char_budget_with(8192, 2048, 26_000);
        assert!(small > mid);
    }

    #[test]
    fn long_question_and_house_rules_take_file_room_on_a_4k_window() {
        let roomy = prompt_budget(4096, 768, 9_000, 0, 0);
        assert!(roomy.retrieval_chars > 0);
        let tight = prompt_budget(4096, 768, 9_000, 4_000, 12_000);
        assert_eq!(tight.retrieval_chars, 0);
        assert!(tight.user_chars < 12_000);
        assert_eq!(tight.history_chars, 0);
        assert!(tight.user_chars > 0);
    }
}
