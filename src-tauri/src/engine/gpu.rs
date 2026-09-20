//! Pick a faster llama.cpp build when the GPU is actually there.
//!
//! The installer still ships the portable pin (Metal / Vulkan / CPU). CUDA
//! and Adreno OpenCL archives are downloaded on first warmup only.

use super::pin::{optional_pin_for, EnginePin};

/// Optional GPU pin for this machine, if a faster archive exists and the
/// hardware can load it.
pub fn preferred_optional_pin() -> Option<&'static EnginePin> {
    let accel = detected_accelerator()?;
    optional_pin_for(std::env::consts::OS, std::env::consts::ARCH, accel)
}

fn detected_accelerator() -> Option<&'static str> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        // The x64 Windows copy on an ARM PC must not chase NVIDIA or Adreno.
        // Vulkan offload on Adreno hangs; CUDA is not present.
        if windows_host_is_arm64() {
            return None;
        }
        if nvidia_driver_present() {
            return Some("CUDA");
        }
    }
    #[cfg(all(windows, target_arch = "aarch64"))]
    if adreno_present() {
        return Some("OpenCL");
    }
    None
}

/// True on Windows ARM, including an x64 process running under emulation.
pub(crate) fn windows_host_is_arm64() -> bool {
    #[cfg(not(windows))]
    {
        false
    }
    #[cfg(all(windows, target_arch = "aarch64"))]
    {
        true
    }
    #[cfg(all(windows, not(target_arch = "aarch64")))]
    {
        native_machine_is_arm64() || cpu_looks_like_snapdragon()
    }
}

#[cfg(windows)]
fn system32() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    std::path::PathBuf::from(root).join("System32")
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn nvidia_driver_present() -> bool {
    let dir = system32();
    dir.join("nvcuda.dll").is_file() || dir.join("nvml.dll").is_file()
}

#[cfg(all(windows, target_arch = "aarch64"))]
fn adreno_present() -> bool {
    if cpu_looks_like_snapdragon() {
        return true;
    }
    let dir = system32();
    ["qcdx11umd64.dll", "qcdx12umd64.dll", "qcdx12.dll"]
        .iter()
        .any(|name| dir.join(name).is_file())
}

#[cfg(windows)]
fn cpu_looks_like_snapdragon() -> bool {
    use sysinfo::CpuRefreshKind;

    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_list(CpuRefreshKind::nothing());
    let brand = sys
        .cpus()
        .first()
        .map(|cpu| cpu.brand().to_ascii_lowercase())
        .unwrap_or_default();
    brand.contains("qualcomm") || brand.contains("snapdragon") || brand.contains("x elite")
}

#[cfg(all(windows, not(target_arch = "aarch64")))]
fn native_machine_is_arm64() -> bool {
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;
    let mut process_machine = 0u16;
    let mut native_machine = 0u16;
    // SAFETY: kernel32, this process, and two stack u16 out-params.
    let ok = unsafe {
        IsWow64Process2(
            GetCurrentProcess(),
            &mut process_machine,
            &mut native_machine,
        )
    };
    ok != 0 && native_machine == IMAGE_FILE_MACHINE_ARM64
}

#[cfg(all(windows, not(target_arch = "aarch64")))]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn IsWow64Process2(
        hProcess: *mut std::ffi::c_void,
        pProcessMachine: *mut u16,
        pNativeMachine: *mut u16,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_pin_is_absent_on_this_host_or_well_formed() {
        if let Some(pin) = preferred_optional_pin() {
            assert!(pin.accelerator == "CUDA" || pin.accelerator == "OpenCL");
            assert_eq!(pin.os, std::env::consts::OS);
            assert_eq!(pin.arch, std::env::consts::ARCH);
        }
    }

    #[test]
    fn macos_is_not_windows_arm() {
        #[cfg(not(windows))]
        assert!(!windows_host_is_arm64());
        #[cfg(all(windows, target_arch = "aarch64"))]
        assert!(windows_host_is_arm64());
    }
}
