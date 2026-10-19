use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};

use crate::error::{Result, ResultExt};

/// Compteurs mémoire d'un processus, en octets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryInfo {
    /// Mémoire privée engagée (« Private bytes ») : la mesure des budgets.
    pub private_bytes: u64,
    pub working_set: u64,
    pub peak_working_set: u64,
}

/// Compteurs du processus courant.
pub fn current_memory() -> Result<MemoryInfo> {
    // SAFETY: pseudo-handle du processus courant, toujours valide, ne se ferme pas.
    query_memory(unsafe { GetCurrentProcess() })
}

/// Compteurs d'un autre processus (même utilisateur).
pub fn memory_of(pid: u32) -> Result<MemoryInfo> {
    // SAFETY: OpenProcess n'a pas de précondition ; le handle est fermé par `OwnedHandle`.
    let h = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
    }
    .op("OpenProcess")?;
    let h = OwnedHandle(h);
    query_memory(h.0)
}

fn query_memory(process: HANDLE) -> Result<MemoryInfo> {
    let mut c = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: `c` est une PROCESS_MEMORY_COUNTERS_EX dont `cb` indique la taille réelle ;
    // la structure étendue commence par les champs de PROCESS_MEMORY_COUNTERS.
    unsafe {
        GetProcessMemoryInfo(
            process,
            (&raw mut c).cast::<PROCESS_MEMORY_COUNTERS>(),
            c.cb,
        )
    }
    .op("GetProcessMemoryInfo")?;
    Ok(MemoryInfo {
        private_bytes: c.PrivateUsage as u64,
        working_set: c.WorkingSetSize as u64,
        peak_working_set: c.PeakWorkingSetSize as u64,
    })
}

/// Nombre de threads d'un processus (instantané Toolhelp). `0` si le processus est introuvable.
pub fn thread_count(pid: u32) -> Result<u32> {
    // SAFETY: pas de précondition ; le handle est fermé par `OwnedHandle`.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .op("CreateToolhelp32Snapshot")?;
    let snap = OwnedHandle(snap);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: handle d'instantané valide ; `entry.dwSize` est initialisé comme exigé.
    let mut next = unsafe { Process32FirstW(snap.0, &raw mut entry) };
    while next.is_ok() {
        if entry.th32ProcessID == pid {
            return Ok(entry.cntThreads);
        }
        // SAFETY: idem.
        next = unsafe { Process32NextW(snap.0, &raw mut entry) };
    }
    Ok(0)
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: handle réel (pas un pseudo-handle) possédé par cette structure.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_has_memory() {
        let m = current_memory().expect("memory");
        assert!(m.private_bytes > 0 && m.working_set > 0);
    }

    #[test]
    fn other_process_by_pid() {
        let m = memory_of(std::process::id()).expect("memory_of");
        assert!(m.private_bytes > 0);
        assert!(thread_count(std::process::id()).expect("threads") >= 1);
    }
}
