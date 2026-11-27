//! Windows Job Object 沙箱：把每次 Agent 运行放进独立作业对象。
//!
//! - `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`：句柄关闭时内核强制终止整棵进程树，
//!   应用崩溃或退出也不会留下继续改文件、继续计费的孤儿进程。
//! - `JOB_OBJECT_LIMIT_PROCESS_MEMORY` / `ACTIVE_PROCESS`：限制单进程内存与进程数，
//!   防止失控 Agent 把机器拖垮。
//!
//! AppContainer 文件级隔离（Phase 1）：
//! - `appcontainer`：profile 与 SID 生命周期；
//! - `acl`：工作目录/可执行文件的 DACL 授予与回收；
//! - `relay`：容器内 TCP ↔ 宿主命名管道；
//! - `host`：以容器身份启动 relay 与 CLI；
//! - `probe`：端到端自检（设置页可触发）。

#[cfg(windows)]
pub mod acl;
#[cfg(windows)]
pub mod appcontainer;
#[cfg(windows)]
pub mod exec;
#[cfg(windows)]
pub mod host;
#[cfg(windows)]
pub mod mock_llm;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
pub mod relay;
#[cfg(windows)]
pub mod session;

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// 单个进程的内存上限（4 GiB），足够常规构建与 Node 工具链。
    pub const MAX_MEMORY_BYTES: usize = 4 * 1024 * 1024 * 1024;
    /// 作业内最多同时存在的进程数，避免 fork 炸弹。
    pub const MAX_PROCESSES: u32 = 64;

    pub struct JobHandle {
        handle: HANDLE,
    }

    // HANDLE 本身是裸指针，但作业对象句柄可跨线程使用（内核对象带引用计数）。
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl JobHandle {
        /// 把子进程加入作业；失败时返回 false（例如父进程已在禁止嵌套的作业里）。
        pub fn assign(&self, pid: u32) -> bool {
            unsafe {
                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if process.is_null() {
                    return false;
                }
                let ok = AssignProcessToJobObject(self.handle, process) != 0;
                CloseHandle(process);
                ok
            }
        }

        /// 立即终止作业内全部进程，用于用户取消或预算硬停。
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.handle, 1);
            }
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            unsafe {
                TerminateJobObject(self.handle, 1);
                CloseHandle(self.handle);
            }
        }
    }

    pub fn create_job() -> Option<JobHandle> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            info.BasicLimitInformation.ActiveProcessLimit = MAX_PROCESSES;
            info.ProcessMemoryLimit = MAX_MEMORY_BYTES;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return None;
            }
            Some(JobHandle { handle: job })
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct JobHandle;

    impl JobHandle {
        pub fn assign(&self, _pid: u32) -> bool {
            false
        }

        pub fn terminate(&self) {}
    }

    pub fn create_job() -> Option<JobHandle> {
        None
    }
}

pub use imp::{create_job, JobHandle};
