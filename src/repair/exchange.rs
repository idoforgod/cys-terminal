//! Rule-one filesystem primitives (RULINGS E5–E7, E9, F3). `exchange` calls
//! `crate::app_bundle::swap_paths` — the bundle install's `renamex_np(RENAME_SWAP)` core, opened for
//! shared use by ONE visibility-only change (F3); this module names no other symbol of that file.
//! Any error of any primitive DECLINES and changes nothing (E9): there is no fallback to a
//! rename-over, ever.

use super::types::{Identity, RepairError, TargetState};
use std::path::Path;

/// The errno names E9 lists (and the others the measurement met), for the decline sentence.
fn errno_name(e: &std::io::Error) -> String {
    #[cfg(unix)]
    {
        if let Some(code) = e.raw_os_error() {
            let name = match code {
                libc::EXDEV => "EXDEV",
                libc::ENOTSUP => "ENOTSUP",
                libc::EACCES => "EACCES",
                libc::EPERM => "EPERM",
                libc::ENOENT => "ENOENT",
                libc::EROFS => "EROFS",
                libc::EEXIST => "EEXIST",
                libc::EINVAL => "EINVAL",
                libc::EBUSY => "EBUSY",
                libc::ENOTDIR => "ENOTDIR",
                libc::EISDIR => "EISDIR",
                libc::ENAMETOOLONG => "ENAMETOOLONG",
                _ => "",
            };
            if !name.is_empty() {
                return name.to_string();
            }
            return format!("errno {code}");
        }
    }
    format!("{:?}", e.kind())
}

/// E9: every error of a rule-one primitive is a DECLINE (nothing changed) with one sentence.
pub fn decline_from_io(what: &str, path: &Path, e: &std::io::Error) -> RepairError {
    RepairError::Declined(format!(
        "{what} 거절 — {}: {} ({e}) · 아무것도 바꾸지 않았습니다",
        path.display(),
        errno_name(e)
    ))
}

/// Atomically swaps the live path with a vault slot entry (`renamex_np(RENAME_SWAP)` through the
/// bundle install's `swap_paths`, macOS). A file, a symbolic link (the link itself — its target is
/// never followed) or one of each. Fails → nothing changed → `Declined`. On Windows there is no
/// exchange: the deposit-first form is used (C12, D:836) and this declines.
pub fn exchange(live: &Path, slot: &Path) -> Result<(), RepairError> {
    #[cfg(unix)]
    {
        crate::app_bundle::swap_paths(live, slot)
            .map_err(|e| decline_from_io("원자적 맞바꾸기", live, &e))
    }
    #[cfg(not(unix))]
    {
        let _ = (live, slot);
        Err(RepairError::Declined(
            "이 운영체제에는 원자적 맞바꾸기가 없습니다 — 먼저 넣는 꼴만 씁니다".into(),
        ))
    }
}

/// E7: is there anything at `dst` — the entry itself (`lstat`, so a dangling link counts), or on
/// any volume another entry of the same folder whose name folds to the same name (a case variant:
/// measured, `RENAME_EXCL` onto a case variant RENAMED that entry instead of failing — Mb row 2d').
fn occupied(dst: &Path) -> Result<Option<String>, RepairError> {
    match std::fs::symlink_metadata(dst) {
        Ok(_) => return Ok(Some(dst.display().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(decline_from_io("자리 확인", dst, &e)),
    }
    let (Some(parent), Some(name)) = (dst.parent(), dst.file_name().and_then(|n| n.to_str())) else {
        return Err(RepairError::Declined(format!(
            "{}: 놓을 자리의 이름을 읽을 수 없습니다",
            dst.display()
        )));
    };
    let want = super::types::fold_name(name);
    let rd = match std::fs::read_dir(parent) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(decline_from_io("자리 확인(폴더 없음)", parent, &e))
        }
        Err(e) => return Err(decline_from_io("자리 확인", parent, &e)),
    };
    for entry in rd.filter_map(Result::ok) {
        if let Some(other) = entry.file_name().to_str() {
            if other != name && super::types::fold_name(other) == want {
                return Ok(Some(parent.join(other).display().to_string()));
            }
        }
    }
    Ok(None)
}

/// No-overwrite placement (RULINGS E7): `lstat` of the destination and a case-fold scan of its
/// folder FIRST — anything there → `Superseded` (a conflict; nothing changed) — then a rename that
/// fails when the destination exists (`renamex_np(RENAME_EXCL)` on macOS; `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING` on Windows); `EEXIST` → conflict. Used for every move INTO the vault
/// (deposit by move) and every placement of a copy at an absent live path.
pub fn place_no_replace(src: &Path, dst: &Path) -> Result<(), RepairError> {
    if let Some(found) = occupied(dst)? {
        return Err(RepairError::Superseded(format!(
            "{}: 그 자리에 이미 무엇인가 있습니다({found}) — 덮어쓰지 않습니다",
            dst.display()
        )));
    }
    rename_excl(src, dst)
}

#[cfg(target_os = "macos")]
fn rename_excl(src: &Path, dst: &Path) -> Result<(), RepairError> {
    use std::os::unix::ffi::OsStrExt;
    let cs = std::ffi::CString::new(src.as_os_str().as_bytes())
        .map_err(|_| RepairError::Declined(format!("{}: NUL in path", src.display())))?;
    let cd = std::ffi::CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| RepairError::Declined(format!("{}: NUL in path", dst.display())))?;
    // SAFETY: two NUL-terminated C strings alive for the call; the kernel reports by rc + errno.
    let rc = unsafe { libc::renamex_np(cs.as_ptr(), cd.as_ptr(), libc::RENAME_EXCL) };
    if rc == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EEXIST) {
        return Err(RepairError::Superseded(format!(
            "{}: 그 자리에 이미 무엇인가 있습니다(EEXIST) — 덮어쓰지 않습니다",
            dst.display()
        )));
    }
    Err(decline_from_io("덮어쓰지 않는 옮기기", dst, &e))
}

#[cfg(windows)]
fn rename_excl(src: &Path, dst: &Path) -> Result<(), RepairError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let w = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
    let (ws, wd) = (w(src), w(dst));
    // SAFETY: two NUL-terminated wide strings alive for the call; flags 0 = never replace.
    let ok = unsafe { MoveFileExW(ws.as_ptr(), wd.as_ptr(), 0) };
    if ok != 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    // ERROR_FILE_EXISTS (80) / ERROR_ALREADY_EXISTS (183)
    if matches!(e.raw_os_error(), Some(80) | Some(183)) {
        return Err(RepairError::Superseded(format!(
            "{}: 그 자리에 이미 무엇인가 있습니다 — 덮어쓰지 않습니다",
            dst.display()
        )));
    }
    Err(decline_from_io("덮어쓰지 않는 옮기기", dst, &e))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn rename_excl(src: &Path, dst: &Path) -> Result<(), RepairError> {
    let _ = src;
    Err(RepairError::Declined(format!(
        "{}: 이 운영체제에서는 덮어쓰지 않는 옮기기를 지원하지 않습니다",
        dst.display()
    )))
}

/// E5: the REAL device id of a directory (`getattrlist(ATTR_CMN_DEVID, FSOPT_RETURN_REALDEV)`,
/// symbolic links in the path followed) — `st_dev` alone is not used: the sealed system volume and
/// the Data volume report the same `st_dev` (measured, Mb §1 item 4).
#[cfg(target_os = "macos")]
fn real_device_id(dir: &Path) -> Result<u64, RepairError> {
    use std::os::unix::ffi::OsStrExt;
    #[repr(C, packed(4))]
    struct Buf {
        len: u32,
        dev: libc::dev_t,
    }
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| RepairError::Declined(format!("{}: NUL in path", dir.display())))?;
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.commonattr = libc::ATTR_CMN_DEVID;
    let mut buf = Buf { len: 0, dev: 0 };
    // SAFETY: valid C string, attrlist and a buffer of the stated size alive for the call.
    let rc = unsafe {
        libc::getattrlist(
            c.as_ptr(),
            &mut al as *mut libc::attrlist as *mut libc::c_void,
            &mut buf as *mut Buf as *mut libc::c_void,
            std::mem::size_of::<Buf>(),
            libc::FSOPT_RETURN_REALDEV,
        )
    };
    if rc != 0 {
        return Err(decline_from_io(
            "같은 파일시스템 확인",
            dir,
            &std::io::Error::last_os_error(),
        ));
    }
    let dev = buf.dev;
    Ok(dev as u32 as u64)
}

/// Real-device comparison of two PARENT directories (RULINGS E5) — `Ok(false)` = different
/// filesystems (decline, change nothing, report). Windows: the volume serial numbers
/// (`GetVolumeInformationByHandleW`, O-C2-5) — an error is a conservative `false`.
pub fn same_filesystem(a_parent: &Path, b_parent: &Path) -> Result<bool, RepairError> {
    #[cfg(target_os = "macos")]
    {
        Ok(real_device_id(a_parent)? == real_device_id(b_parent)?)
    }
    #[cfg(windows)]
    {
        Ok(match (volume_serial(a_parent), volume_serial(b_parent)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        })
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        use std::os::unix::fs::MetadataExt;
        let a = std::fs::metadata(a_parent).map_err(|e| decline_from_io("같은 파일시스템 확인", a_parent, &e))?;
        let b = std::fs::metadata(b_parent).map_err(|e| decline_from_io("같은 파일시스템 확인", b_parent, &e))?;
        Ok(a.dev() == b.dev())
    }
}

#[cfg(windows)]
fn open_for_query(p: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(p)
}

#[cfg(windows)]
fn volume_serial(p: &Path) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
    let f = open_for_query(p).ok()?;
    let mut serial: u32 = 0;
    // SAFETY: a valid handle; null name/fs buffers with zero sizes; the serial out-pointer is valid.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            f.as_raw_handle() as _,
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    (ok != 0).then_some(serial)
}

/// The volume UUID of the volume holding `dir` (`statfs` → mount point → `getattrlist`
/// `ATTR_VOL_UUID`) — stable across re-attach, unlike `st_dev` (E6).
#[cfg(target_os = "macos")]
fn volume_uuid(dir: &Path) -> Result<String, RepairError> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| RepairError::Superseded(format!("{}: NUL in path", dir.display())))?;
    let mut sf: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid C string and statfs buffer.
    if unsafe { libc::statfs(c.as_ptr(), &mut sf) } != 0 {
        return Err(RepairError::Superseded(format!(
            "{}: 볼륨을 확인할 수 없습니다({})",
            dir.display(),
            std::io::Error::last_os_error()
        )));
    }
    #[repr(C, packed(4))]
    struct Buf {
        len: u32,
        uuid: [u8; 16],
    }
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID;
    let mut buf = Buf {
        len: 0,
        uuid: [0; 16],
    };
    // SAFETY: f_mntonname is NUL-terminated by the kernel; buffer sizes as stated.
    let rc = unsafe {
        libc::getattrlist(
            sf.f_mntonname.as_ptr(),
            &mut al as *mut libc::attrlist as *mut libc::c_void,
            &mut buf as *mut Buf as *mut libc::c_void,
            std::mem::size_of::<Buf>(),
            0,
        )
    };
    if rc != 0 {
        return Err(RepairError::Superseded(format!(
            "{}: 볼륨 UUID 를 확인할 수 없습니다({})",
            dir.display(),
            std::io::Error::last_os_error()
        )));
    }
    let uuid = buf.uuid;
    if uuid.iter().all(|b| *b == 0) {
        return Err(RepairError::Superseded(format!(
            "{}: 볼륨 UUID 가 비어 있습니다",
            dir.display()
        )));
    }
    Ok(uuid.iter().map(|b| format!("{b:02x}")).collect())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// (volume id, inode / file index) of the ENTRY itself (a link is not followed).
pub fn entry_identity(p: &Path) -> Result<(String, u64), RepairError> {
    let unconfirmable = |why: String| {
        RepairError::Superseded(format!(
            "{}: 정체(볼륨·아이노드)를 확인할 수 없습니다 — {why}",
            p.display()
        ))
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(p).map_err(|e| unconfirmable(e.to_string()))?;
        let parent = p
            .parent()
            .ok_or_else(|| unconfirmable("부모 폴더 없음".into()))?;
        #[cfg(target_os = "macos")]
        let vol = volume_uuid(parent)?;
        #[cfg(not(target_os = "macos"))]
        let vol = {
            let pm = std::fs::metadata(parent).map_err(|e| unconfirmable(e.to_string()))?;
            format!("dev-{:x}", pm.dev())
        };
        Ok((vol, m.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let f = open_for_query(p).map_err(|e| unconfirmable(e.to_string()))?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid handle and an out struct of the right type.
        let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) };
        if ok == 0 {
            return Err(unconfirmable(std::io::Error::last_os_error().to_string()));
        }
        Ok((
            format!("{:08x}", info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
}

/// Volume UUID + inode + content hash (RULINGS E6). For a symbolic link the hash is the sha256 of
/// its target state (`link:<target>`). An identity that cannot be confirmed is `Superseded` — the
/// safe direction (E6: "unconfirmable → superseded").
pub fn identity_of(p: &Path) -> Result<Identity, RepairError> {
    let (volume_uuid, inode) = entry_identity(p)?;
    let sha256 = match TargetState::read(p) {
        Ok(TargetState::File { sha256, .. }) => sha256,
        Ok(TargetState::Link { target }) => sha256_hex(format!("link:{target}").as_bytes()),
        Ok(TargetState::Absent) => {
            return Err(RepairError::Superseded(format!(
                "{}: 그 자리에 아무것도 없습니다",
                p.display()
            )))
        }
        Err(e) => {
            return Err(RepairError::Superseded(format!(
                "{}: 내용을 확인할 수 없습니다 — {e:?}",
                p.display()
            )))
        }
    };
    // the identity must not have moved while the hash was read
    if entry_identity(p)? != (volume_uuid.clone(), inode) {
        return Err(RepairError::Superseded(format!(
            "{}: 읽는 동안 바뀌었습니다",
            p.display()
        )));
    }
    Ok(Identity {
        volume_uuid,
        inode,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cys-exchange-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn exchange_swaps_files_and_links_with_their_metadata() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("swap");
        std::fs::create_dir_all(d.join("live")).unwrap();
        std::fs::create_dir_all(d.join("slot")).unwrap();
        let live = d.join("live/hook.sh");
        let slot = d.join("slot/hook.sh");
        std::fs::write(&live, b"OLD").unwrap();
        std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(&slot, b"NEW").unwrap();
        std::fs::set_permissions(&slot, std::fs::Permissions::from_mode(0o755)).unwrap();
        exchange(&live, &slot).unwrap();
        assert_eq!(std::fs::read(&live).unwrap(), b"NEW");
        assert_eq!(std::fs::read(&slot).unwrap(), b"OLD");
        // E2: the mode travels with the content
        assert_eq!(
            std::fs::symlink_metadata(&live).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        // file <-> link: the link itself is exchanged, its target untouched
        let l = d.join("slot/link");
        std::os::unix::fs::symlink("/nonexistent/target", &l).unwrap();
        exchange(&live, &l).unwrap();
        assert_eq!(
            std::fs::read_link(&live).unwrap(),
            std::path::PathBuf::from("/nonexistent/target")
        );
        assert_eq!(std::fs::read(&l).unwrap(), b"NEW");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// E9: a failing exchange declines and changes nothing (one side missing → ENOENT).
    #[cfg(unix)]
    #[test]
    fn exchange_failure_declines_and_changes_nothing() {
        let d = tmp("fail");
        let live = d.join("a");
        std::fs::write(&live, b"A").unwrap();
        let e = exchange(&live, &d.join("missing")).unwrap_err();
        assert!(matches!(e, RepairError::Declined(ref m) if m.contains("ENOENT") || m.contains("Unsupported")), "{e:?}");
        assert_eq!(std::fs::read(&live).unwrap(), b"A");
        assert!(!d.join("missing").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// E9: EVERY listed errno maps to a decline with its name (no fallback path exists).
    #[cfg(unix)]
    #[test]
    fn every_listed_errno_declines() {
        for (code, name) in [
            (libc::EXDEV, "EXDEV"),
            (libc::ENOTSUP, "ENOTSUP"),
            (libc::EACCES, "EACCES"),
            (libc::EPERM, "EPERM"),
            (libc::ENOENT, "ENOENT"),
            (libc::EROFS, "EROFS"),
        ] {
            let e = decline_from_io("x", Path::new("/p"), &std::io::Error::from_raw_os_error(code));
            assert!(matches!(e, RepairError::Declined(ref m) if m.contains(name)), "{name}: {e:?}");
        }
    }

    /// E7: lstat first (incl. a dangling link and a case variant), then RENAME_EXCL.
    #[cfg(target_os = "macos")]
    #[test]
    fn place_no_replace_refuses_existing_and_case_variants() {
        let d = tmp("place");
        let src = d.join("src");
        std::fs::write(&src, b"S").unwrap();
        // existing entry
        std::fs::write(d.join("dst"), b"D").unwrap();
        assert!(matches!(place_no_replace(&src, &d.join("dst")), Err(RepairError::Superseded(_))));
        // dangling link counts as occupied
        std::os::unix::fs::symlink("/nonexistent", d.join("dl")).unwrap();
        assert!(matches!(place_no_replace(&src, &d.join("dl")), Err(RepairError::Superseded(_))));
        // case variant of an existing entry — refused whatever the volume's case rule
        std::fs::write(d.join("hook.sh"), b"H").unwrap();
        assert!(matches!(place_no_replace(&src, &d.join("HOOK.sh")), Err(RepairError::Superseded(_))));
        assert_eq!(std::fs::read(d.join("hook.sh")).unwrap(), b"H");
        assert!(src.exists(), "nothing moved on a conflict");
        // absent destination → moved
        place_no_replace(&src, &d.join("fresh")).unwrap();
        assert!(!src.exists());
        assert_eq!(std::fs::read(d.join("fresh")).unwrap(), b"S");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn same_filesystem_by_real_device_of_parents() {
        let d = tmp("samefs");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        assert!(same_filesystem(&d.join("a"), &d.join("b")).unwrap());
        // the root of the sealed system volume has another REAL device than a temp folder (E5)
        assert!(!same_filesystem(Path::new("/usr/bin"), &d).unwrap());
        assert!(same_filesystem(&d.join("missing"), &d).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// E6: (volume UUID, inode) + hash; a replaced entry has another identity; unconfirmable →
    /// Superseded.
    #[cfg(unix)]
    #[test]
    fn identity_is_volume_inode_and_hash() {
        let d = tmp("ident");
        let p = d.join("f");
        std::fs::write(&p, b"one").unwrap();
        let a = identity_of(&p).unwrap();
        assert!(!a.volume_uuid.is_empty());
        let b = identity_of(&p).unwrap();
        assert_eq!(a, b);
        std::fs::write(d.join("g"), b"one").unwrap();
        std::fs::rename(d.join("g"), &p).unwrap();
        let c = identity_of(&p).unwrap();
        assert_eq!(c.sha256, a.sha256, "same bytes");
        assert_ne!(c.inode, a.inode, "another writer's identical bytes are another identity");
        assert!(matches!(identity_of(&d.join("absent")), Err(RepairError::Superseded(_))));
        let _ = std::fs::remove_dir_all(&d);
    }
}
