//! Four mediated project tools. Writes require a bound, one-use Main approval.
//! Exchange preserves the displaced inode; external editors can still race.
//! An ambiguous observation stops the run and retains both versions.
use super::effects_store::{Binding, Entry, Journal, Record};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CStr, CString},
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, RawFd},
    path::{Path, PathBuf},
};

pub(crate) use super::effects_store::Binding as CallBinding;
const LIMIT: usize = 1024 * 1024;
const RESERVED: &str = ".lomi-effect-";
fn failure() -> String {
    "The project tool could not establish its approved file state. No automatic replay is allowed."
        .into()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect::<std::collections::BTreeMap<_, _>>();
            serde_json::to_value(sorted).unwrap()
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        value => value.clone(),
    }
}
pub(super) fn value_digest(value: &Value) -> Result<String, String> {
    Ok(digest(
        &serde_json::to_vec(&canonical(value)).map_err(|_| failure())?,
    ))
}
fn closed(value: &Value, keys: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
    })
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key].as_str().ok_or_else(failure)
}
fn hash_argument(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let hash = value
        .as_str()
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(failure)?;
    Ok(Some(hash.into()))
}
pub(crate) fn tool_specs() -> Vec<Value> {
    let path = json!({"type":"string","minLength":1,"maxLength":4096,"description":"Relative granted-project path; symlinks and router state are unavailable."});
    let hash = json!({"anyOf":[{"type":"string","pattern":"^[0-9a-f]{64}$"},{"type":"null"}],"description":"Exact SHA256 from lomi_read; null only for creating an absent file."});
    vec![
        json!({"type":"function","name":"lomi_list","description":"List one project directory without following links.","inputSchema":{"type":"object","properties":{"path":{"type":"string","minLength":0,"maxLength":4096}},"required":["path"],"additionalProperties":false}}),
        json!({"type":"function","name":"lomi_read","description":"Read at most 1 MiB of UTF-8 project text and its SHA256.","inputSchema":{"type":"object","properties":{"path":path.clone()},"required":["path"],"additionalProperties":false}}),
        json!({"type":"function","name":"lomi_write","description":"Prepare a complete UTF-8 file replacement for explicit one-use approval.","inputSchema":{"type":"object","properties":{"path":path.clone(),"expectedSha256":hash.clone(),"content":{"type":"string","maxLength":LIMIT}},"required":["path","expectedSha256","content"],"additionalProperties":false}}),
        json!({"type":"function","name":"lomi_apply_patch","description":"Prepare sequential exact unique-text edits for explicit approval.","inputSchema":{"type":"object","properties":{"path":path,"expectedSha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},"edits":{"type":"array","minItems":1,"maxItems":128,"items":{"type":"object","properties":{"oldText":{"type":"string","minLength":1},"newText":{"type":"string"}},"required":["oldText","newText"],"additionalProperties":false}}},"required":["path","expectedSha256","edits"],"additionalProperties":false}}),
    ]
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunBinding {
    pub run_id: String,
    pub attempt_id: String,
    pub generation: u64,
    pub auth_revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Approval {
    pub binding: Binding,
    pub nonce: String,
    pub args_digest: String,
    pub before_hash: Option<String>,
    pub after_hash: String,
    pub result_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Preview {
    pub binding: Binding,
    pub path: String,
    pub args_digest: String,
    pub before_hash: Option<String>,
    pub after_hash: String,
    pub result_digest: String,
    pub before_bytes: usize,
    pub after_bytes: usize,
    pub before_text: Option<String>,
    pub after_text: String,
    pub recovery_path: String,
}
pub(crate) enum Prepared {
    Result(Value),
    ApprovalRequired(Box<Preview>),
}
pub(crate) enum Recovery {
    Committed {
        binding: Box<Binding>,
        result: Value,
    },
    Uncertain {
        binding: Box<Binding>,
        path: String,
        recovery_path: String,
    },
    Unapplied {
        binding: Box<Binding>,
        path: String,
        recovery_path: String,
    },
}
impl Recovery {
    /// Check the retained evidence before native history may continue.
    pub(crate) fn validate_committed(&self, run_id: &str) -> Result<(), String> {
        let binding = match self {
            Self::Committed { binding, .. }
            | Self::Uncertain { binding, .. }
            | Self::Unapplied { binding, .. } => binding,
        };
        binding.key()?;
        if binding.run_id != run_id {
            return Err(
                "Retained project recovery belongs to another run. Native continuity is fenced."
                    .into(),
            );
        }
        match self {
            Self::Committed { result, .. } => {
                if !result.is_object() {
                    return Err("The retained project result is invalid. Native continuity is fenced.".into());
                }
                value_digest(result)?;
                Ok(())
            }
            Self::Uncertain { path, recovery_path, .. }
            | Self::Unapplied { path, recovery_path, .. } => Err(format!(
                "Project effect for {path:?} requires explicit recovery; retained version: {recovery_path:?}. No native task will be replayed."
            )),
        }
    }
}
pub(crate) struct Broker {
    journal: Journal,
    root: File,
    root_path: PathBuf,
    denied: Vec<PathBuf>,
    denied_identities: Vec<(u64, u64)>,
    run: RunBinding,
    identity: (u64, u64),
}
/// Native OS account home, never an inherited environment override.
pub(crate) fn account_home() -> Result<PathBuf, String> {
    let mut capacity = 16 * 1024;
    loop {
        let mut bytes = vec![0u8; capacity];
        let mut account = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let error = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                account.as_mut_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE && capacity < 1024 * 1024 {
            capacity *= 2;
            continue;
        }
        if error != 0 || result.is_null() {
            return Err(failure());
        }
        let account = unsafe { account.assume_init() };
        if account.pw_dir.is_null() {
            return Err(failure());
        }
        let home = unsafe { CStr::from_ptr(account.pw_dir) }
            .to_str()
            .map_err(|_| failure())?;
        if home.is_empty() {
            return Err(failure());
        }
        let home = PathBuf::from(home).canonicalize().map_err(|_| failure())?;
        if !home.is_absolute() || !home.is_dir() {
            return Err(failure());
        }
        return Ok(home);
    }
}
fn sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".env")
        || matches!(
            lower.as_str(),
            ".git"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".gnupg"
                | ".codex"
                | ".netrc"
                | ".npmrc"
                | ".pypirc"
                | "credentials"
                | "credentials.json"
                | "id_rsa"
                | "id_ed25519"
                | "auth.json"
                | "tokens"
                | "tokens.json"
                | "token.json"
                | "api_key"
                | "api-key"
                | "apikey"
                | "secrets"
                | "secrets.json"
        )
        || [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}
fn sync_known(parent: RawFd, name: &str, hash: &str, expected: (u64, u64)) -> Result<(), String> {
    let mut file = fd_open(parent, name, libc::O_RDONLY | libc::O_NONBLOCK)?;
    let info = stat(&file)?;
    if info.st_mode & libc::S_IFMT != libc::S_IFREG
        || info.st_nlink != 1
        || identity(&file)? != expected
    {
        return Err(failure());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > LIMIT || digest(&bytes) != hash {
        return Err(failure());
    }
    file.sync_all().map_err(|_| failure())
}
fn cstring(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| failure())
}
fn fd_open(parent: RawFd, name: &str, flags: i32) -> Result<File, String> {
    let name = cstring(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(failure());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn stat(file: &File) -> Result<libc::stat, String> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(failure());
    }
    Ok(unsafe { stat.assume_init() })
}
fn device_id<T: TryInto<u64>>(value: T) -> Result<u64, String> {
    value.try_into().map_err(|_| failure())
}
fn identity(file: &File) -> Result<(u64, u64), String> {
    let stat = stat(file)?;
    Ok((device_id(stat.st_dev)?, stat.st_ino))
}
fn read_file(parent: RawFd, name: &str) -> Result<Option<(String, libc::stat)>, String> {
    let name_c = cstring(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name_c.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            return Ok(None);
        }
        return Err(failure());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|_| failure())?;
    let info = stat(&file)?;
    if info.st_mode & libc::S_IFMT != libc::S_IFREG
        || info.st_nlink != 1
        || info.st_size < 0
        || info.st_size as u64 > LIMIT as u64
    {
        return Err(failure());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > LIMIT {
        return Err(failure());
    }
    let after = stat(&file)?;
    if file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|_| failure())?
        != modified
        || info.st_dev != after.st_dev
        || info.st_ino != after.st_ino
        || info.st_size != after.st_size
        || after.st_nlink != 1
    {
        return Err(failure());
    }
    Ok(Some((
        String::from_utf8(bytes).map_err(|_| failure())?,
        info,
    )))
}
impl Broker {
    pub(crate) fn open(
        private_run_root: &Path,
        granted_project: &Path,
        denied_router_roots: &[PathBuf],
        run: RunBinding,
    ) -> Result<Self, String> {
        let root_path = granted_project.canonicalize().map_err(|_| failure())?;
        if root_path.components().any(|component|matches!(component,std::path::Component::Normal(name) if name.to_str().is_none_or(sensitive))){return Err(failure())}
        let home = account_home()?;
        if root_path == Path::new("/") || home.starts_with(&root_path) {
            return Err(failure());
        }
        if root_path != granted_project
            || std::fs::symlink_metadata(granted_project)
                .map_err(|_| failure())?
                .file_type()
                .is_symlink()
        {
            return Err(failure());
        }
        let root = fd_open(
            libc::AT_FDCWD,
            root_path.to_str().ok_or_else(failure)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        let root_identity = identity(&root)?;
        let mut denied = vec![private_run_root.canonicalize().map_err(|_| failure())?];
        for path in denied_router_roots {
            denied.push(path.canonicalize().map_err(|_| failure())?)
        }
        if denied
            .iter()
            .any(|path| root_path.starts_with(path) || path.starts_with(&root_path))
        {
            return Err(failure());
        }
        denied.sort();
        denied.dedup();
        let mut denied_identities = Vec::new();
        for path in &denied {
            if std::fs::metadata(path).map_err(|_| failure())?.is_dir() {
                let directory = fd_open(
                    libc::AT_FDCWD,
                    path.to_str().ok_or_else(failure)?,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                )?;
                denied_identities.push(identity(&directory)?);
            }
        }
        let mut journal = Journal::open(private_run_root)?;
        journal.bind_project(&serde_json::to_string(&json!({"run":run.run_id,"root":root_path,"device":root_identity.0,"inode":root_identity.1,"denied":denied})).map_err(|_|failure())?)?;
        Ok(Self {
            journal,
            root,
            root_path,
            denied,
            denied_identities,
            run,
            identity: root_identity,
        })
    }
    fn fence(&self, binding: &Binding) -> Result<(), String> {
        binding.key()?;
        if binding.run_id != self.run.run_id
            || binding.attempt_id != self.run.attempt_id
            || binding.generation != self.run.generation
            || binding.auth_revision != self.run.auth_revision
        {
            return Err(failure());
        }
        let current = fd_open(
            libc::AT_FDCWD,
            self.root_path.to_str().ok_or_else(failure)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        if identity(&self.root)? != self.identity || identity(&current)? != self.identity {
            return Err(failure());
        }
        Ok(())
    }
    fn components<'a>(&self, path: &'a str) -> Result<Vec<&'a str>, String> {
        if path.is_empty()
            || path.len() > 4096
            || path.starts_with('/')
            || path.contains('\\')
            || path.chars().any(char::is_control)
        {
            return Err(failure());
        }
        let parts = path.split('/').collect::<Vec<_>>();
        if parts.len() > 128
            || parts.iter().any(|part| {
                part.is_empty()
                    || *part == "."
                    || *part == ".."
                    || part.starts_with(RESERVED)
                    || sensitive(part)
            })
        {
            return Err(failure());
        }
        let absolute = self.root_path.join(path);
        if self
            .denied
            .iter()
            .any(|denied| absolute.starts_with(denied))
        {
            return Err(failure());
        }
        Ok(parts)
    }
    fn parent(&self, path: &str) -> Result<(File, String), String> {
        let parts = self.components(path)?;
        let mut parent = self.root.try_clone().map_err(|_| failure())?;
        for part in &parts[..parts.len() - 1] {
            parent = fd_open(parent.as_raw_fd(), part, libc::O_RDONLY | libc::O_DIRECTORY)?;
            if self.denied_identities.contains(&identity(&parent)?) {
                return Err(failure());
            }
        }
        Ok((parent, parts[parts.len() - 1].into()))
    }
    fn list(&self, path: &str) -> Result<Value, String> {
        let directory = if path == "." || path.is_empty() {
            fd_open(
                self.root.as_raw_fd(),
                ".",
                libc::O_RDONLY | libc::O_DIRECTORY,
            )?
        } else {
            let (parent, name) = self.parent(path)?;
            fd_open(
                parent.as_raw_fd(),
                &name,
                libc::O_RDONLY | libc::O_DIRECTORY,
            )?
        };
        if self.denied_identities.contains(&identity(&directory)?) {
            return Err(failure());
        }
        let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(failure());
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            unsafe { libc::close(duplicate) };
            return Err(failure());
        }
        struct Dir(*mut libc::DIR);
        impl Drop for Dir {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let guard = Dir(stream);
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        loop {
            #[cfg(target_os = "linux")]
            let error_pointer = unsafe { libc::__errno_location() };
            #[cfg(target_os = "macos")]
            let error_pointer = unsafe { libc::__error() };
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return Err(failure());
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            unsafe {
                *error_pointer = 0
            };
            let entry = unsafe { libc::readdir(guard.0) };
            if entry.is_null() {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if unsafe { *error_pointer } != 0 {
                    return Err(failure());
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| failure())?;
            if name == "." || name == ".." || name.starts_with(RESERVED) || sensitive(name) {
                continue;
            }
            if name.chars().any(char::is_control) {
                return Err(failure());
            }
            bytes = bytes.saturating_add(name.len() + 128);
            if bytes > LIMIT || entries.len() >= 4096 {
                return Err(failure());
            }
            let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
            let name_c = cstring(name)?;
            if unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    name_c.as_ptr(),
                    info.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(failure());
            }
            let info = unsafe { info.assume_init() };
            let kind = match info.st_mode & libc::S_IFMT {
                libc::S_IFREG if info.st_nlink == 1 => "file",
                libc::S_IFDIR => "directory",
                _ => continue,
            };
            let absolute = self.root_path.join(if path == "." || path.is_empty() {
                name.to_string()
            } else {
                format!("{path}/{name}")
            });
            if self
                .denied
                .iter()
                .any(|denied| absolute.starts_with(denied))
            {
                continue;
            }
            entries.push(json!({"name":name,"kind":kind}));
        }
        entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(json!({"path":path,"entries":entries}))
    }
    pub(crate) fn prepare(
        &mut self,
        binding: Binding,
        name: &str,
        args: &Value,
    ) -> Result<Prepared, String> {
        self.fence(&binding)?;
        if !self.journal.pending()?.is_empty() {
            return Err(failure());
        }
        if serde_json::to_vec(args).map_err(|_| failure())?.len() > 2 * LIMIT {
            return Err(failure());
        }
        let args_digest = value_digest(args)?;
        if let Some(entry) = self.journal.get(&binding)? {
            if !(entry.record.name == "lomi_list"
                && (entry.record.path.is_empty() || entry.record.path == "."))
            {
                self.components(&entry.record.path)?;
            }
            if entry.record.args_digest != args_digest
                || entry.record.name != name
                || entry.record.binding.key()? != binding.key()?
            {
                return Err(failure());
            }
            return match entry.state.as_str() {
                "committed" => Ok(Prepared::Result(entry.record.result)),
                "denied" => Ok(Prepared::Result(self.journal.denied_result(&binding)?)),
                "prepared" if entry.record.binding == binding => Ok(Prepared::ApprovalRequired(
                    Box::new(self.preview(&entry.record)?),
                )),
                _ => Err(failure()),
            };
        }
        let keys = match name {
            "lomi_list" | "lomi_read" => vec!["path"],
            "lomi_write" => vec!["path", "expectedSha256", "content"],
            "lomi_apply_patch" => vec!["path", "expectedSha256", "edits"],
            _ => return Err(failure()),
        };
        if !closed(args, &keys) {
            return Err(failure());
        }
        let path = text(args, "path")?;
        if name == "lomi_list" {
            let result = self.list(path)?;
            if serde_json::to_vec(&result).map_err(|_| failure())?.len() > LIMIT {
                return Err(failure());
            }
            let (dev, ino) = self.identity;
            let record = Record {
                binding,
                name: name.into(),
                args_digest,
                path: path.into(),
                before_hash: None,
                after_hash: None,
                before_dev: None,
                before_ino: None,
                parent_dev: dev,
                parent_ino: ino,
                before_bytes: None,
                after_bytes: None,
                temporary: None,
                result_digest: value_digest(&result)?,
                result: result.clone(),
            };
            self.journal.insert(&record, "committed")?;
            return Ok(Prepared::Result(result));
        }
        let (parent, leaf) = self.parent(path)?;
        let current = read_file(parent.as_raw_fd(), &leaf)?;
        let before_hash = current.as_ref().map(|(text, _)| digest(text.as_bytes()));
        let (dev, ino) = identity(&parent)?;
        if name == "lomi_read" {
            let (content, _) = current.as_ref().ok_or_else(failure)?;
            let result =
                json!({"path":path,"content":content,"sha256":before_hash,"bytes":content.len()});
            if serde_json::to_vec(&result).map_err(|_| failure())?.len() > LIMIT {
                return Err(failure());
            }
            let record = Record {
                binding,
                name: name.into(),
                args_digest,
                path: path.into(),
                before_hash,
                after_hash: None,
                before_dev: None,
                before_ino: None,
                parent_dev: dev,
                parent_ino: ino,
                before_bytes: None,
                after_bytes: None,
                temporary: None,
                result_digest: value_digest(&result)?,
                result: result.clone(),
            };
            self.journal.insert(&record, "committed")?;
            return Ok(Prepared::Result(result));
        }
        if hash_argument(&args["expectedSha256"])? != before_hash {
            return Err(
                "The project file changed. Read it again before preparing a new edit.".into(),
            );
        }
        let after: String = if name == "lomi_write" {
            text(args, "content")?.into()
        } else {
            let mut content = current.as_ref().ok_or_else(failure)?.0.clone();
            let edits = args["edits"]
                .as_array()
                .filter(|edits| !edits.is_empty() && edits.len() <= 128)
                .ok_or_else(failure)?;
            for edit in edits {
                if !closed(edit, &["oldText", "newText"]) {
                    return Err(failure());
                }
                let old = text(edit, "oldText")?;
                let new = text(edit, "newText")?;
                if old.is_empty()
                    || content
                        .char_indices()
                        .filter(|(index, _)| content[*index..].starts_with(old))
                        .take(2)
                        .count()
                        != 1
                {
                    return Err("Each patch oldText must occur exactly once in sequence.".into());
                }
                content = content.replacen(old, new, 1);
                if content.len() > LIMIT {
                    return Err(failure());
                }
            }
            content
        };
        if after.len() > LIMIT || after.contains('\0') {
            return Err(failure());
        }
        let after_hash = digest(after.as_bytes());
        let result = json!({"path":path,"sha256":after_hash,"bytes":after.len()});
        let (before_dev, before_ino) = current
            .as_ref()
            .map(|(_, stat)| -> Result<_, String> {
                Ok((Some(device_id(stat.st_dev)?), Some(stat.st_ino)))
            })
            .transpose()?
            .unwrap_or((None, None));
        let record = Record {
            binding,
            name: name.into(),
            args_digest,
            path: path.into(),
            before_hash,
            after_hash: Some(after_hash),
            before_dev,
            before_ino,
            parent_dev: dev,
            parent_ino: ino,
            before_bytes: current.as_ref().map(|(text, _)| text.clone()),
            after_bytes: Some(after),
            temporary: Some(format!("{RESERVED}{}", super::new_id()?)),
            result_digest: value_digest(&result)?,
            result,
        };
        self.journal.insert(&record, "prepared")?;
        Ok(Prepared::ApprovalRequired(Box::new(self.preview(&record)?)))
    }
    fn preview(&self, record: &Record) -> Result<Preview, String> {
        let (parent, leaf) = self.parent(&record.path)?;
        let before = read_file(parent.as_raw_fd(), &leaf)?;
        if before.as_ref().map(|(text, _)| digest(text.as_bytes())) != record.before_hash {
            return Err(failure());
        }
        Ok(Preview {
            binding: record.binding.clone(),
            path: record.path.clone(),
            args_digest: record.args_digest.clone(),
            before_hash: record.before_hash.clone(),
            after_hash: record.after_hash.clone().ok_or_else(failure)?,
            result_digest: record.result_digest.clone(),
            before_bytes: before.as_ref().map_or(0, |(text, _)| text.len()),
            after_bytes: record.after_bytes.as_ref().ok_or_else(failure)?.len(),
            before_text: record.before_bytes.clone(),
            after_text: record.after_bytes.clone().ok_or_else(failure)?,
            recovery_path: recovery_path(record)?,
        })
    }
    pub(crate) fn deny(&mut self, binding: &Binding) -> Result<Value, String> {
        self.fence(binding)?;
        self.journal.deny(binding)
    }
    pub(crate) fn apply_approved(
        &mut self,
        approval: &Approval,
        mut owner_fence: impl FnMut() -> Result<(), String>,
    ) -> Result<Value, String> {
        self.fence(&approval.binding)?;
        if !self.journal.pending()?.is_empty() {
            return Err(failure());
        }
        let entry = self.journal.get(&approval.binding)?.ok_or_else(failure)?;
        let record = &entry.record;
        if entry.state != "prepared"
            || record.binding != approval.binding
            || record.args_digest != approval.args_digest
            || record.before_hash != approval.before_hash
            || record.after_hash.as_ref() != Some(&approval.after_hash)
            || record.result_digest != approval.result_digest
        {
            return Err(failure());
        }
        let (parent, leaf) = self.parent(&record.path)?;
        if identity(&parent)? != (record.parent_dev, record.parent_ino) {
            return Err(failure());
        }
        let before = read_file(parent.as_raw_fd(), &leaf)?;
        if !matches_before(record, before.as_ref()) {
            return Err(failure());
        }
        let temporary = record.temporary.as_deref().ok_or_else(failure)?;
        let bytes = record.after_bytes.as_ref().ok_or_else(failure)?.as_bytes();
        if digest(bytes) != approval.after_hash
            || value_digest(&record.result)? != approval.result_digest
        {
            return Err(failure());
        }
        owner_fence()?;
        self.fence(&approval.binding)?;
        self.journal.authorize(
            &approval.binding,
            &approval.nonce,
            &value_digest(&serde_json::to_value(approval).map_err(|_| failure())?)?,
        )?;
        let attempt = (|| -> Result<(), String> {
            let mut target = fd_open(
                parent.as_raw_fd(),
                temporary,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            )?;
            if let Some((_, info)) = before.as_ref() {
                if unsafe { libc::fchmod(target.as_raw_fd(), info.st_mode & 0o777) } != 0 {
                    return Err(failure());
                }
            }
            target
                .write_all(bytes)
                .and_then(|_| target.sync_all())
                .map_err(|_| failure())?;
            let staged = identity(&target)?;
            self.journal.stage(&approval.binding, staged.0, staged.1)?;
            if !matches_before(record, read_file(parent.as_raw_fd(), &leaf)?.as_ref()) {
                return Err(failure());
            }
            owner_fence()?;
            self.fence(&approval.binding)?;
            if identity(&self.parent(&record.path)?.0)? != identity(&parent)? {
                return Err(failure());
            }
            exchange(
                parent.as_raw_fd(),
                temporary,
                &leaf,
                record.before_hash.is_some(),
            )?;
            if record.before_hash.is_some() {
                sync_known(
                    parent.as_raw_fd(),
                    temporary,
                    record.before_hash.as_deref().ok_or_else(failure)?,
                    (
                        record.before_dev.ok_or_else(failure)?,
                        record.before_ino.ok_or_else(failure)?,
                    ),
                )?;
            }
            parent.sync_all().map_err(|_| failure())?;
            self.fence(&approval.binding)?;
            if identity(&self.parent(&record.path)?.0)? != identity(&parent)?
                || !matches_after(
                    record,
                    read_file(parent.as_raw_fd(), &leaf)?.as_ref(),
                    Some(staged),
                )
            {
                return Err(failure());
            }
            let backup = read_file(parent.as_raw_fd(), temporary)?;
            if if record.before_hash.is_some() {
                !matches_before(record, backup.as_ref())
            } else {
                backup.is_some()
            } {
                return Err(failure());
            }
            Ok(())
        })();
        if attempt.is_err() {
            let _ = self
                .journal
                .transition(&approval.binding, "executing", "uncertain");
            return Err(failure());
        }
        self.journal
            .transition(&approval.binding, "executing", "committed")?;
        // Displaced originals remain reserved in the project until the owner
        // durably checkpoints the native result. There is no eager deletion.
        Ok(record.result.clone())
    }
    pub(crate) fn recover(&mut self) -> Result<Vec<Recovery>, String> {
        let current = fd_open(
            libc::AT_FDCWD,
            self.root_path.to_str().ok_or_else(failure)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        if identity(&current)? != self.identity {
            return Err(failure());
        }
        let mut results = Vec::new();
        for Entry { record, state } in self.journal.pending()? {
            let staged = self.journal.staged(&record.binding)?;
            let inspected = (|| -> Result<u8, String> {
                let (parent, leaf) = self.parent(&record.path)?;
                if identity(&parent)? != (record.parent_dev, record.parent_ino) {
                    return Err(failure());
                }
                let target = read_file(parent.as_raw_fd(), &leaf)?;
                let backup = read_file(
                    parent.as_raw_fd(),
                    record.temporary.as_deref().ok_or_else(failure)?,
                )?;
                if matches_after(&record, target.as_ref(), staged)
                    && if record.before_hash.is_some() {
                        matches_before(&record, backup.as_ref())
                    } else {
                        backup.is_none()
                    }
                {
                    // Observation alone is not a durable rename. Sync both
                    // retained versions and directory, then inspect again
                    // before an acknowledged committed journal transition.
                    sync_known(
                        parent.as_raw_fd(),
                        &leaf,
                        record.after_hash.as_deref().ok_or_else(failure)?,
                        staged.ok_or_else(failure)?,
                    )?;
                    if record.before_hash.is_some() {
                        sync_known(
                            parent.as_raw_fd(),
                            record.temporary.as_deref().ok_or_else(failure)?,
                            record.before_hash.as_deref().ok_or_else(failure)?,
                            (
                                record.before_dev.ok_or_else(failure)?,
                                record.before_ino.ok_or_else(failure)?,
                            ),
                        )?;
                    }
                    parent.sync_all().map_err(|_| failure())?;
                    if identity(&self.parent(&record.path)?.0)?
                        != (record.parent_dev, record.parent_ino)
                        || !matches_after(
                            &record,
                            read_file(parent.as_raw_fd(), &leaf)?.as_ref(),
                            staged,
                        )
                        || (record.before_hash.is_some()
                            && !matches_before(
                                &record,
                                read_file(
                                    parent.as_raw_fd(),
                                    record.temporary.as_deref().ok_or_else(failure)?,
                                )?
                                .as_ref(),
                            ))
                    {
                        return Err(failure());
                    }
                    if record.before_hash.is_none()
                        && read_file(
                            parent.as_raw_fd(),
                            record.temporary.as_deref().ok_or_else(failure)?,
                        )?
                        .is_some()
                    {
                        return Err(failure());
                    }
                    return Ok(1);
                }
                if matches_before(&record, target.as_ref())
                    && if staged.is_some() {
                        matches_after(&record, backup.as_ref(), staged)
                    } else {
                        backup.is_none()
                    }
                {
                    return Ok(2);
                }
                Ok(0)
            })();
            if inspected == Ok(1) {
                self.journal
                    .transition(&record.binding, &state, "committed")?;
                results.push(Recovery::Committed {
                    binding: Box::new(record.binding),
                    result: record.result,
                });
            } else if inspected == Ok(2) {
                self.journal
                    .transition(&record.binding, &state, "unapplied")?;
                results.push(Recovery::Unapplied {
                    binding: Box::new(record.binding.clone()),
                    path: record.path.clone(),
                    recovery_path: recovery_path(&record)?,
                });
            } else {
                if state == "executing" {
                    self.journal
                        .transition(&record.binding, "executing", "uncertain")?;
                }
                results.push(Recovery::Uncertain {
                    binding: Box::new(record.binding.clone()),
                    path: record.path.clone(),
                    recovery_path: recovery_path(&record)?,
                });
            }
        }
        Ok(results)
    }
}
fn matches_before(record: &Record, current: Option<&(String, libc::stat)>) -> bool {
    match current {
        None => record.before_hash.is_none(),
        Some((text, stat)) => {
            record.before_hash.as_deref() == Some(digest(text.as_bytes()).as_str())
                && device_id(stat.st_dev)
                    .ok()
                    .is_some_and(|dev| record.before_dev == Some(dev))
                && record.before_ino == Some(stat.st_ino)
        }
    }
}
fn matches_after(
    record: &Record,
    current: Option<&(String, libc::stat)>,
    staged: Option<(u64, u64)>,
) -> bool {
    current.is_some_and(|(text, stat)| {
        record.after_hash.as_deref() == Some(digest(text.as_bytes()).as_str())
            && device_id(stat.st_dev)
                .ok()
                .is_some_and(|dev| staged == Some((dev, stat.st_ino)))
    })
}
fn recovery_path(record: &Record) -> Result<String, String> {
    let parent = record
        .path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    Ok(format!(
        "{parent}{}{}",
        if parent.is_empty() { "" } else { "/" },
        record.temporary.as_deref().ok_or_else(failure)?
    ))
}
fn exchange(parent: RawFd, source: &str, target: &str, existing: bool) -> Result<(), String> {
    let source = cstring(source)?;
    let target = cstring(target)?;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            parent,
            source.as_ptr(),
            parent,
            target.as_ptr(),
            if existing { 2u32 } else { 1u32 },
        )
    };
    #[cfg(target_os = "macos")]
    let result = {
        extern "C" {
            fn renameatx_np(
                from: i32,
                source: *const libc::c_char,
                to: i32,
                target: *const libc::c_char,
                flags: u32,
            ) -> i32;
        }
        unsafe {
            renameatx_np(
                parent,
                source.as_ptr(),
                parent,
                target.as_ptr(),
                if existing { 0x2 } else { 0x4 },
            )
        }
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let result = -1;
    if result != 0 {
        return Err("This filesystem did not confirm atomic project exchange/create. Both versions are retained; review before continuing.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding(call: &str) -> Binding {
        Binding {
            run_id: "run".into(),
            attempt_id: "attempt".into(),
            generation: 1,
            auth_revision: 2,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            call_id: call.into(),
        }
    }
    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Broker) {
        let journal = tempfile::tempdir().unwrap();
        crate::chat::storage::private(journal.path(), true).unwrap();
        let project = tempfile::tempdir().unwrap();
        let project_path = project.path().canonicalize().unwrap();
        let broker = Broker::open(
            journal.path(),
            &project_path,
            &[],
            RunBinding {
                run_id: "run".into(),
                attempt_id: "attempt".into(),
                generation: 1,
                auth_revision: 2,
            },
        )
        .unwrap();
        (journal, project, broker)
    }
    fn approval(preview: &Preview) -> Approval {
        Approval {
            binding: preview.binding.clone(),
            nonce: "a".repeat(64),
            args_digest: preview.args_digest.clone(),
            before_hash: preview.before_hash.clone(),
            after_hash: preview.after_hash.clone(),
            result_digest: preview.result_digest.clone(),
        }
    }
    fn prepared(
        broker: &mut Broker,
        call: &str,
        path: &str,
        before: Option<&str>,
        after: &str,
    ) -> Preview {
        match broker.prepare(binding(call),"lomi_write",&json!({"path":path,"expectedSha256":before.map(|before|digest(before.as_bytes())),"content":after})).unwrap(){Prepared::ApprovalRequired(preview)=>*preview,_=>panic!("write must require approval")}
    }
    #[test]
    fn denied_reply_is_durable_and_replays_across_owner_generations_without_success() {
        let (journal, project, mut broker) = fixture();
        let args = json!({"path":"file","expectedSha256":null,"content":"proposed"});
        let preview = prepared(&mut broker, "deny", "file", None, "proposed");
        let intent = broker
            .journal
            .get(&preview.binding)
            .unwrap()
            .unwrap()
            .record;
        let denied = broker.deny(&preview.binding).unwrap();
        assert_eq!(
            denied,
            json!({"denied":true,"path":"file","reason":"The user denied this coding effect."})
        );
        assert_eq!(broker.deny(&preview.binding).unwrap(), denied);
        let retained = broker
            .journal
            .get(&preview.binding)
            .unwrap()
            .unwrap()
            .record;
        assert_eq!(retained.result, intent.result);
        assert_eq!(retained.result_digest, intent.result_digest);
        assert!(!project.path().join("file").exists());
        drop(broker);
        let mut broker = Broker::open(
            journal.path(),
            &project.path().canonicalize().unwrap(),
            &[],
            RunBinding {
                run_id: "run".into(),
                attempt_id: "next".into(),
                generation: 3,
                auth_revision: 4,
            },
        )
        .unwrap();
        let mut next = preview.binding;
        next.attempt_id = "next".into();
        next.generation = 3;
        next.auth_revision = 4;
        match broker.prepare(next.clone(), "lomi_write", &args).unwrap() {
            Prepared::Result(result) => assert_eq!(result, denied),
            _ => panic!("denial must not ask for fresh consent"),
        }
        assert!(broker
            .prepare(
                next,
                "lomi_write",
                &json!({"path":"file","expectedSha256":null,"content":"different"})
            )
            .is_err());
        assert!(!project.path().join("file").exists());
    }
    #[test]
    fn legacy_or_corrupted_denial_never_replays_the_proposed_success() {
        for legacy in [true, false] {
            let (journal, project, mut broker) = fixture();
            let preview = prepared(&mut broker, "deny", "file", None, "proposed");
            broker.deny(&preview.binding).unwrap();
            drop(broker);
            let connection =
                rusqlite::Connection::open(journal.path().join("effects.sqlite3")).unwrap();
            if legacy {
                connection
                    .execute_batch("DROP TABLE denials;PRAGMA user_version=1;")
                    .unwrap();
            } else {
                connection
                    .execute("UPDATE denials SET result_digest=?", ["0".repeat(64)])
                    .unwrap();
            }
            drop(connection);
            let mut broker = Broker::open(
                journal.path(),
                &project.path().canonicalize().unwrap(),
                &[],
                RunBinding {
                    run_id: "run".into(),
                    attempt_id: "attempt".into(),
                    generation: 1,
                    auth_revision: 2,
                },
            )
            .unwrap();
            assert!(broker
                .prepare(
                    preview.binding,
                    "lomi_write",
                    &json!({"path":"file","expectedSha256":null,"content":"proposed"})
                )
                .is_err());
            assert!(!project.path().join("file").exists());
        }
    }
    #[test]
    fn read_call_replays_exact_checkpoint_and_conflicts_never_reread() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "original").unwrap();
        let args = json!({"path":"file"});
        let first = match broker.prepare(binding("read"), "lomi_read", &args).unwrap() {
            Prepared::Result(value) => value,
            _ => panic!(),
        };
        std::fs::write(project.path().join("file"), "changed").unwrap();
        let second = match broker.prepare(binding("read"), "lomi_read", &args).unwrap() {
            Prepared::Result(value) => value,
            _ => panic!(),
        };
        assert_eq!(first, second);
        assert!(broker
            .prepare(binding("read"), "lomi_read", &json!({"path":"another"}))
            .is_err());
        let mut next_attempt = binding("read");
        next_attempt.attempt_id = "other".into();
        assert!(broker.prepare(next_attempt, "lomi_read", &args).is_err());
    }
    #[test]
    fn distinct_root_list_calls_each_open_a_fresh_directory_description() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("first"), "one").unwrap();
        std::fs::write(project.path().join("second"), "two").unwrap();
        let first = match broker
            .prepare(binding("list-one"), "lomi_list", &json!({"path":"."}))
            .unwrap()
        {
            Prepared::Result(value) => value,
            _ => panic!(),
        };
        let second = match broker
            .prepare(binding("list-two"), "lomi_list", &json!({"path":"."}))
            .unwrap()
        {
            Prepared::Result(value) => value,
            _ => panic!(),
        };
        assert_eq!(first, second);
        assert_eq!(first["entries"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn sensitive_paths_are_hidden_and_native_namespace_overlap_is_rejected() {
        let (journal, project, mut broker) = fixture();
        for (index, name) in [
            ".env.production",
            ".NETRC",
            "credentials.json",
            "server.pem",
            "tokens.json",
        ]
        .iter()
        .enumerate()
        {
            std::fs::write(project.path().join(name), "credential fixture").unwrap();
            assert!(broker
                .prepare(
                    binding(&format!("secret-{index}")),
                    "lomi_read",
                    &json!({"path":name})
                )
                .is_err());
        }
        std::fs::write(project.path().join("visible.txt"), "public").unwrap();
        let list = match broker
            .prepare(binding("empty-root-path"), "lomi_list", &json!({"path":""}))
            .unwrap()
        {
            Prepared::Result(value) => value,
            _ => panic!(),
        };
        assert_eq!(
            list["entries"],
            json!([{"name":"visible.txt","kind":"file"}])
        );
        let config = project.path().join("native-config");
        std::fs::create_dir(&config).unwrap();
        assert!(Broker::open(
            journal.path(),
            &project.path().canonicalize().unwrap(),
            &[config],
            broker.run.clone()
        )
        .is_err());
        assert!(Broker::open(
            journal.path(),
            &account_home().unwrap(),
            &[],
            broker.run.clone()
        )
        .is_err());
    }
    #[test]
    fn swapped_fifo_sync_open_is_nonblocking_and_refuses_special_inode() {
        let (_journal, project, broker) = fixture();
        let fifo = project.path().join("fifo");
        let name = CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(sync_known(
            broker.root.as_raw_fd(),
            "fifo",
            &digest(b"approved"),
            (0, 0)
        )
        .is_err());
    }
    #[test]
    fn hash_cas_and_current_owner_prevent_stale_approval_effects() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "original").unwrap();
        let preview = prepared(&mut broker, "write", "file", Some("original"), "approved");
        assert_eq!(preview.before_text.as_deref(), Some("original"));
        assert_eq!(preview.after_text, "approved");
        assert!(broker
            .apply_approved(&approval(&preview), || Err("cancelled".into()))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(project.path().join("file")).unwrap(),
            "original"
        );
        std::fs::write(project.path().join("file"), "editor changed").unwrap();
        assert!(broker
            .apply_approved(&approval(&preview), || Ok(()))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(project.path().join("file")).unwrap(),
            "editor changed"
        );
    }
    #[test]
    fn approved_exchange_is_one_use_and_replays_only_its_recorded_result() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "original").unwrap();
        let preview = prepared(
            &mut broker,
            "write-once",
            "file",
            Some("original"),
            "approved",
        );
        let consent = approval(&preview);
        let mut fences = 0;
        let result = broker
            .apply_approved(&consent, || {
                fences += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(fences, 2);
        assert_eq!(
            std::fs::read_to_string(project.path().join("file")).unwrap(),
            "approved"
        );
        assert_eq!(
            std::fs::read_to_string(project.path().join(&preview.recovery_path)).unwrap(),
            "original"
        );
        assert!(broker.apply_approved(&consent, || Ok(())).is_err());
        std::fs::write(project.path().join("file"), "later editor text").unwrap();
        let replay = broker
            .prepare(
                binding("write-once"),
                "lomi_write",
                &json!({"path":"file","expectedSha256":digest(b"original"),"content":"approved"}),
            )
            .unwrap();
        assert!(matches!(replay,Prepared::Result(recorded) if recorded==result));
        assert_eq!(
            std::fs::read_to_string(project.path().join("file")).unwrap(),
            "later editor text"
        );
    }
    #[test]
    fn sequential_patch_rejects_ambiguous_overlapping_occurrences() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "aaa").unwrap();
        assert!(broker.prepare(binding("ambiguous"),"lomi_apply_patch",&json!({"path":"file","expectedSha256":digest(b"aaa"),"edits":[{"oldText":"aa","newText":"b"}]})).is_err());
        std::fs::write(project.path().join("file"), "one two").unwrap();
        let result=broker.prepare(binding("sequential"),"lomi_apply_patch",&json!({"path":"file","expectedSha256":digest(b"one two"),"edits":[{"oldText":"one","newText":"three"},{"oldText":"three two","newText":"done"}]})).unwrap();
        match result {
            Prepared::ApprovalRequired(preview) => assert_eq!(preview.after_text, "done"),
            _ => panic!(),
        }
    }
    #[test]
    fn symlinks_traversal_hardlinks_and_router_journal_are_unavailable() {
        use std::os::unix::fs::symlink;
        let (journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "text").unwrap();
        symlink(project.path().join("file"), project.path().join("link")).unwrap();
        std::fs::hard_link(project.path().join("file"), project.path().join("hard")).unwrap();
        for (index, path) in [
            "../file",
            "/etc/passwd",
            "link",
            "hard",
            ".lomi-effect-user",
            "file/../file",
        ]
        .iter()
        .enumerate()
        {
            assert!(broker
                .prepare(
                    binding(&format!("denied{index}")),
                    "lomi_read",
                    &json!({"path":path})
                )
                .is_err());
        }
        assert!(Broker::open(
            journal.path(),
            &journal.path().canonicalize().unwrap(),
            &[],
            broker.run.clone()
        )
        .is_err());
    }
    fn staged_swap(broker: &mut Broker, preview: &Preview) -> (Record, File, String) {
        let record = broker
            .journal
            .get(&preview.binding)
            .unwrap()
            .unwrap()
            .record;
        let (parent, leaf) = broker.parent(&record.path).unwrap();
        broker
            .journal
            .authorize(
                &record.binding,
                &"b".repeat(64),
                &digest(b"approval fixture"),
            )
            .unwrap();
        let mut temporary = fd_open(
            parent.as_raw_fd(),
            record.temporary.as_deref().unwrap(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )
        .unwrap();
        temporary
            .write_all(record.after_bytes.as_ref().unwrap().as_bytes())
            .unwrap();
        temporary.sync_all().unwrap();
        let (dev, ino) = identity(&temporary).unwrap();
        broker.journal.stage(&record.binding, dev, ino).unwrap();
        (record, parent, leaf)
    }
    #[test]
    fn crash_after_exchange_recovers_only_both_original_and_new_inode_proofs() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "original").unwrap();
        let preview = prepared(&mut broker, "crash", "file", Some("original"), "approved");
        let (record, parent, leaf) = staged_swap(&mut broker, &preview);
        exchange(
            parent.as_raw_fd(),
            record.temporary.as_deref().unwrap(),
            &leaf,
            true,
        )
        .unwrap();
        parent.sync_all().unwrap();
        let recovery = broker.recover().unwrap();
        assert!(
            matches!(recovery.as_slice(),[Recovery::Committed{result,..}] if result["sha256"]==digest(b"approved"))
        );
        recovery[0]
            .validate_committed(&preview.binding.run_id)
            .unwrap();
        assert!(recovery[0].validate_committed("another-run").is_err());
        assert!(broker
            .apply_approved(&approval(&preview), || Ok(()))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(project.path().join(preview.recovery_path)).unwrap(),
            "original"
        );
    }
    #[test]
    fn changed_displaced_backup_makes_recovery_uncertain_and_blocks_new_calls() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "original").unwrap();
        let preview = prepared(&mut broker, "crash", "file", Some("original"), "approved");
        let (record, parent, leaf) = staged_swap(&mut broker, &preview);
        exchange(
            parent.as_raw_fd(),
            record.temporary.as_deref().unwrap(),
            &leaf,
            true,
        )
        .unwrap();
        std::fs::rename(
            project.path().join(&preview.recovery_path),
            project.path().join("held-original-backup"),
        )
        .unwrap();
        std::fs::write(project.path().join(&preview.recovery_path), "original").unwrap();
        assert!(matches!(
            broker.recover().unwrap().as_slice(),
            [Recovery::Uncertain { .. }]
        ));
        assert!(broker
            .prepare(binding("next"), "lomi_read", &json!({"path":"file"}))
            .is_err());
    }
    #[test]
    fn create_race_and_failed_atomic_primitive_never_overwrite_a_file() {
        let (_journal, project, mut broker) = fixture();
        let preview = prepared(&mut broker, "new", "file", None, "approved");
        let (record, parent, leaf) = staged_swap(&mut broker, &preview);
        std::fs::write(project.path().join("file"), "concurrent creation").unwrap();
        assert!(exchange(
            parent.as_raw_fd(),
            record.temporary.as_deref().unwrap(),
            &leaf,
            false
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(project.path().join("file")).unwrap(),
            "concurrent creation"
        );
        // Unsupported/invalid atomic primitives fail closed; there is no plain
        // rename fallback. The retained temp and target remain available.
        assert!(exchange(-1, record.temporary.as_deref().unwrap(), &leaf, true).is_err());
        assert!(project.path().join(&preview.recovery_path).exists());
        assert!(matches!(
            broker.recover().unwrap().as_slice(),
            [Recovery::Uncertain { .. }]
        ));
    }
    #[test]
    fn crash_before_exchange_is_known_unapplied_and_never_reuses_approval() {
        let (_journal, project, mut broker) = fixture();
        std::fs::write(project.path().join("file"), "original").unwrap();
        let preview = prepared(&mut broker, "staged", "file", Some("original"), "approved");
        let _ = staged_swap(&mut broker, &preview);
        let recovery = broker.recover().unwrap();
        assert!(matches!(recovery.as_slice(), [Recovery::Unapplied { .. }]));
        let report = recovery[0]
            .validate_committed(&preview.binding.run_id)
            .unwrap_err();
        assert!(report.contains(&preview.path));
        assert!(report.contains(&preview.recovery_path));
        assert!(broker
            .apply_approved(&approval(&preview), || Ok(()))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(project.path().join("file")).unwrap(),
            "original"
        );
    }
}
