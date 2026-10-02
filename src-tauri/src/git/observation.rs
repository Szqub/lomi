use std::{
    io,
    process::{Child, Command},
};

#[cfg(not(windows))]
pub(super) type Guard = ();
#[cfg(windows)]
pub(super) type Guard = std::os::windows::io::OwnedHandle;

pub(super) fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

#[cfg(target_os = "macos")]
pub(super) fn configured() -> Result<Command, String> {
    use std::{fs, os::unix::fs::MetadataExt, path::PathBuf, sync::OnceLock};
    static GIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let git = GIT
        .get_or_init(|| {
            // Resolve the selected developer tool outside the repository, without
            // inherited DYLD, Git, or developer-directory overrides.
            let output = Command::new("/usr/bin/xcrun")
                .args(["--find", "git"])
                .current_dir("/")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .output()
                .map_err(|error| format!("Cannot locate trusted Git: {error}"))?;
            if !output.status.success() {
                return Err(
                    "Safe Git observations require installed Xcode or Command Line Tools.".into(),
                );
            }
            let path = PathBuf::from(
                String::from_utf8(output.stdout)
                    .map_err(|error| error.to_string())?
                    .trim(),
            );
            let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err("The selected Git executable is not a trusted system tool.".into());
            }
            Ok(path)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    const PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny process-fork)
(deny process-exec)
(allow process-exec (literal (param "GIT_EXEC")))
(deny file-write*)
(allow file-write* (literal "/dev/null"))
"#;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", PROFILE])
        .arg(format!("-DGIT_EXEC={}", git.display()))
        .arg(git);
    Ok(command)
}

#[cfg(target_os = "linux")]
pub(super) fn configured() -> Result<Command, String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("git");
    // Install before exec; a failure returns from spawn without starting Git.
    unsafe {
        command.pre_exec(deny_children);
    }
    Ok(command)
}

#[cfg(target_os = "linux")]
fn deny_children() -> io::Result<()> {
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Err(io::Error::from_raw_os_error(libc::ENOTSUP));
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        // seccomp_data: nr at offset 0, arch at offset 4. Reject other ABIs,
        // including x32, and all process/thread creation syscalls. Git builtin
        // observations never need another executable; mutations use a separate
        // command path. No allocation or locking occurs after fork.
        let stmt = |code, k| libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        };
        let eq = |k| libc::sock_filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k,
        };
        let deny = stmt(0x06, 0x00050000 | libc::EPERM as u32);
        let mut filter = [
            stmt(0x20, 4),
            libc::sock_filter {
                code: 0x15,
                jt: 1,
                jf: 0,
                k: ARCH,
            },
            stmt(0x06, 0x80000000),
            stmt(0x20, 0),
            libc::sock_filter {
                code: 0x35,
                jt: 0,
                jf: 1,
                k: 0x40000000,
            },
            deny,
            eq(libc::SYS_clone as u32),
            deny,
            eq(libc::SYS_clone3 as u32),
            deny,
            #[cfg(target_arch = "x86_64")]
            eq(libc::SYS_fork as u32),
            #[cfg(target_arch = "x86_64")]
            deny,
            #[cfg(target_arch = "x86_64")]
            eq(libc::SYS_vfork as u32),
            #[cfg(target_arch = "x86_64")]
            deny,
            stmt(0x06, 0x7fff0000),
        ];
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_mut_ptr(),
        };
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
            || unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
pub(super) fn configured() -> Result<Command, String> {
    use std::os::windows::process::CommandExt;
    use std::{fs, path::PathBuf, sync::OnceLock};
    static GIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let git = GIT
        .get_or_init(|| {
            let path = std::env::var_os("PATH").ok_or("Git is not installed.")?;
            for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
                let candidate = directory.join("git.exe");
                if !candidate.is_file() {
                    continue;
                }
                // Git for Windows' cmd/bin entry points are launcher executables.
                // Run the actual builtin binary so the single-process job does not
                // need to authorize a launcher child before helper isolation.
                if directory.file_name().is_some_and(|name| {
                    name.to_str().is_some_and(|name| {
                        name.eq_ignore_ascii_case("cmd") || name.eq_ignore_ascii_case("bin")
                    })
                }) {
                    if let Some(installation) = directory.parent() {
                        for architecture in ["ucrt64", "mingw64", "mingw32", "clangarm64"] {
                            let actual = installation.join(architecture).join("bin/git.exe");
                            if actual.is_file() {
                                return fs::canonicalize(actual).map_err(|error| error.to_string());
                            }
                        }
                    }
                }
                return fs::canonicalize(candidate).map_err(|error| error.to_string());
            }
            Err("Cannot locate the installed Git executable.".into())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let mut command = Command::new(git);
    command.creation_flags(0x08000000 | 0x00000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
    Ok(command)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(super) fn configured() -> Result<Command, String> {
    Err("Safe Git observations are unsupported on this platform.".into())
}

#[cfg(not(windows))]
pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Guard)> {
    command.spawn().map(|child| (child, ()))
}

#[cfg(windows)]
pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Guard)> {
    use std::{
        mem::{size_of, zeroed},
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut child = command.spawn()?;
    let protect = || -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut found = None;
        let mut present = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while present != 0 {
            if entry.th32OwnerProcessID == child.id() {
                if found.is_some() {
                    return Err(io::Error::other("Unexpected threads in suspended Git."));
                }
                found = Some(entry.th32ThreadID);
            }
            present = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        let id =
            found.ok_or_else(|| io::Error::other("Cannot locate the suspended Git thread."))?;
        let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, id) };
        if thread.is_null() {
            return Err(io::Error::last_os_error());
        }
        let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
        // No Git code runs before the non-breakaway single-process Job applies.
        if unsafe { ResumeThread(thread.as_raw_handle()) } != 1 {
            return Err(io::Error::other(
                "Cannot safely resume the Git observation.",
            ));
        }
        Ok(())
    };
    if let Err(error) = protect() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok((child, job))
}
