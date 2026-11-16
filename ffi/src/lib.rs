//! C ABI over upstream transports and command functions. See README.md for ownership rules.
use anyhow::Result;
use mcumgr_client as upstream;
use serde::Serialize;
use std::cell::{Cell, RefCell, UnsafeCell};
use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr;
use std::thread::{self, ThreadId};

/// Status: 0 success, 1 invalid argument, 2 operation failure, 3 caught panic.
pub type McStatus = i32;
/// Synchronous image upload callback. Do not reenter or close the connection.
pub type McProgress = Option<unsafe extern "C" fn(u64, u64, *mut c_void)>;

/// Owned bytes, not NUL terminated. Initialize to {NULL, 0}; free with mc_buffer_free.
/// Do not copy ownership, change fields, or reuse a nonempty output buffer.
#[repr(C)]
pub struct McBuffer {
    pub data: *mut u8,
    pub len: usize,
}

/// Opaque connection. Use and close on its creating thread; no concurrent access.
pub struct McConnection {
    owner: ThreadId,
    busy: Cell<bool>,
    transport: UnsafeCell<Box<dyn upstream::Transport>>,
}
#[derive(Debug)]
struct Invalid(&'static str);
impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}
impl std::error::Error for Invalid {}
fn invalid(msg: &'static str) -> anyhow::Error { Invalid(msg).into() }
thread_local! { static ERROR: RefCell<String> = const { RefCell::new(String::new()) }; }
fn boundary(f: impl FnOnce() -> Result<()>) -> McStatus {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => { ERROR.with(|s| s.borrow_mut().clear()); 0 }
        Ok(Err(e)) => {
            let status = if e.downcast_ref::<Invalid>().is_some() { 1 } else { 2 };
            ERROR.with(|s| *s.borrow_mut() = format!("{e:#}"));
            status
        }
        Err(_) => { ERROR.with(|s| *s.borrow_mut() = "Rust panic caught at C ABI boundary".into()); 3 }
    }
}
unsafe fn text<'a>(s: *const c_char) -> Result<&'a str> {
    if s.is_null() { return Err(invalid("required string is NULL")); }
    CStr::from_ptr(s).to_str().map_err(|_| invalid("string is not UTF-8"))
}
unsafe fn opt_text<'a>(s: *const c_char) -> Result<Option<&'a str>> {
    if s.is_null() { Ok(None) } else { text(s).map(Some) }
}
unsafe fn bytes<'a>(p: *const u8, n: usize) -> Result<&'a [u8]> {
    if n > isize::MAX as usize { return Err(invalid("buffer too large")); }
    if n == 0 { return Ok(&[]); }
    if p.is_null() { return Err(invalid("nonempty bytes are NULL")); }
    Ok(std::slice::from_raw_parts(p, n))
}
unsafe fn output(out: *mut McBuffer) -> Result<()> {
    let b = out.as_ref().ok_or_else(|| invalid("output is NULL"))?;
    if !b.data.is_null() || b.len != 0 { return Err(invalid("output must be empty")); }
    Ok(())
}
unsafe fn put(out: *mut McBuffer, data: Vec<u8>) -> Result<()> {
    output(out)?;
    if data.is_empty() { return Ok(()); }
    let b = data.into_boxed_slice();
    let len = b.len();
    (*out).data = Box::into_raw(b) as *mut u8;
    (*out).len = len;
    Ok(())
}
unsafe fn json(out: *mut McBuffer, value: impl Serialize) -> Result<()> {
    put(out, serde_json::to_vec(&value)?)
}
unsafe fn connection<T>(p: *mut McConnection, f: impl FnOnce(&mut dyn upstream::Transport) -> Result<T>) -> Result<T> {
    let c = p.as_ref().ok_or_else(|| invalid("connection is NULL"))?;
    if c.owner != thread::current().id() { return Err(invalid("connection belongs to another thread")); }
    if c.busy.replace(true) { return Err(invalid("connection is busy")); }
    struct Busy<'a>(&'a Cell<bool>);
    impl Drop for Busy<'_> { fn drop(&mut self) { self.0.set(false); } }
    let _busy = Busy(&c.busy);
    f((&mut *c.transport.get()).as_mut())
}
unsafe fn open(out: *mut *mut McConnection, spec: upstream::ConnSpec) -> Result<()> {
    if out.is_null() || !(*out).is_null() { return Err(invalid("connection output must point to NULL")); }
    let transport = spec.open()?;
    *out = Box::into_raw(Box::new(McConnection {
        owner: thread::current().id(), busy: Cell::new(false), transport: UnsafeCell::new(transport),
    }));
    Ok(())
}

/// Open serial. Upstream CLI defaults: 115200 baud, 60 s initial timeout, line 128, MTU 512.
/// "test" selects upstream's simulated serial device.
#[no_mangle]
pub unsafe extern "C" fn mc_open_serial(device: *const c_char, baudrate: u32, initial_timeout_s: u32, linelength: usize, mtu: usize, out: *mut *mut McConnection) -> McStatus {
    boundary(|| {
        if baudrate == 0 || initial_timeout_s == 0 || linelength < 8 || mtu < 128 || mtu > 65535 {
            return Err(invalid("invalid serial baudrate, timeout, line length or MTU"));
        }
        open(out, upstream::ConnSpec::Serial(upstream::SerialSpecs {
            device: text(device)?.into(), baudrate, initial_timeout_s,
            subsequent_timeout_ms: 200, linelength, mtu,
        }))
    })
}
/// Open UDP. Defaults: port 1337, timeout 60 s, MTU 512. IPv6 host must be bracketed.
#[no_mangle]
pub unsafe extern "C" fn mc_open_udp(host: *const c_char, port: u16, timeout_s: u32, mtu: usize, out: *mut *mut McConnection) -> McStatus {
    boundary(|| {
        if port == 0 || timeout_s == 0 || !(128..=65507).contains(&mtu) { return Err(invalid("invalid UDP settings")); }
        open(out, upstream::ConnSpec::Udp(upstream::UdpSpecs { host: text(host)?.into(), port, timeout_s, mtu }))
    })
}
/// Open BLE. Address/name are optional; use upstream discovery when both are NULL.
/// Defaults: scan 10 s, response timeout 60 s, MTU 244.
#[no_mangle]
pub unsafe extern "C" fn mc_open_ble(address: *const c_char, name: *const c_char, scan_timeout_s: u32, timeout_s: u32, mtu: usize, out: *mut *mut McConnection) -> McStatus {
    boundary(|| {
        if scan_timeout_s == 0 || timeout_s == 0 || !(128..=512).contains(&mtu) { return Err(invalid("invalid BLE settings")); }
        open(out, upstream::ConnSpec::Ble(upstream::BleSpecs {
            address: opt_text(address)?.map(str::to_owned), name: opt_text(name)?.map(str::to_owned),
            scan_timeout_s, timeout_s, mtu,
        }))
    })
}
/// Close and set *connection to NULL. NULL *connection is accepted.
#[no_mangle]
pub unsafe extern "C" fn mc_close(p: *mut *mut McConnection) -> McStatus {
    boundary(|| {
        if p.is_null() { return Err(invalid("connection output is NULL")); }
        if (*p).is_null() { return Ok(()); }
        connection(*p, |_| Ok(()))?;
        let owned = Box::from_raw(*p);
        *p = ptr::null_mut();
        drop(owned);
        Ok(())
    })
}
/// Free library-owned bytes and reset the buffer. An empty buffer is accepted.
#[no_mangle]
pub unsafe extern "C" fn mc_buffer_free(b: *mut McBuffer) -> McStatus {
    boundary(|| {
        let b = b.as_mut().ok_or_else(|| invalid("buffer is NULL"))?;
        if b.data.is_null() {
            if b.len != 0 { return Err(invalid("invalid empty buffer")); }
        } else {
            let data = b.data; let len = b.len;
            b.data = ptr::null_mut(); b.len = 0;
            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data, len)));
        }
        Ok(())
    })
}
/// Copy this thread's last error as UTF-8 bytes. Does not clear the error on success.
#[no_mangle]
pub unsafe extern "C" fn mc_last_error(out: *mut McBuffer) -> McStatus {
    let saved = ERROR.with(|s| s.borrow().clone());
    let status = boundary(|| put(out, saved.as_bytes().to_vec()));
    if status == 0 { ERROR.with(|s| *s.borrow_mut() = saved); }
    status
}
/// Return serial port names as a JSON string array.
#[no_mangle]
pub unsafe extern "C" fn mc_serial_ports_json(out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; json(out, serialport::available_ports()?.into_iter().map(|p| p.port_name).collect::<Vec<_>>()) })
}
/// Set transport timeout in milliseconds.
#[no_mangle]
pub unsafe extern "C" fn mc_set_timeout(c: *mut McConnection, timeout_ms: u32) -> McStatus {
    boundary(|| {
        if timeout_ms == 0 { return Err(invalid("timeout must be positive")); }
        connection(c, |t| t.set_timeout(timeout_ms))
    })
}
/// Upload using upstream chunking and retry rules. Defaults: retries=4, subsequent timeout=200 ms.
/// Upstream may override slot from a filename containing slot1/slot3.
#[no_mangle]
pub unsafe extern "C" fn mc_image_upload(c: *mut McConnection, path: *const c_char, slot: u8, retries: u32, subsequent_timeout_ms: u32, progress: McProgress, user: *mut c_void) -> McStatus {
    boundary(|| {
        let path = Path::new(text(path)?);
        if subsequent_timeout_ms == 0 { return Err(invalid("timeout must be positive")); }
        if std::fs::metadata(path)?.len() == 0 { return Err(invalid("image file is empty")); }
        connection(c, |t| upstream::upload_image(t, path, slot, retries, subsequent_timeout_ms,
            Some(|offset, total| { if let Some(cb) = progress { cb(offset, total, user); } })))
    })
}
/// Set image state. confirm=-1 omits the field, 0=false, 1=true.
#[no_mangle]
pub unsafe extern "C" fn mc_image_test(c: *mut McConnection, hash: *const u8, hash_len: usize, confirm: i32) -> McStatus {
    boundary(|| {
        let confirm = match confirm { -1 => None, 0 => Some(false), 1 => Some(true), _ => return Err(invalid("invalid confirm")) };
        let hash = bytes(hash, hash_len)?.to_vec();
        connection(c, |t| upstream::test(t, hash, confirm))
    })
}
/// Erase image. slot=-1 omits the slot field.
#[no_mangle]
pub unsafe extern "C" fn mc_image_erase(c: *mut McConnection, slot: i64) -> McStatus {
    boundary(|| {
        let slot = match slot { -1 => None, 0..=4294967295 => Some(slot as u32), _ => return Err(invalid("invalid slot")) };
        connection(c, |t| upstream::erase(t, slot))
    })
}
/// Raw custom command. op is 0 (read) or 2 (write). Input/output are CBOR, not JSON.
/// Returns response CBOR including any device error fields without interpreting them.
#[no_mangle]
pub unsafe extern "C" fn mc_transceive_cbor(c: *mut McConnection, op: u8, group: u16, id: u8, body: *const u8, body_len: usize, out: *mut McBuffer) -> McStatus {
    boundary(|| {
        output(out)?;
        let op = match op { 0 => upstream::NmpOp::Read, 2 => upstream::NmpOp::Write, _ => return Err(invalid("invalid request op")) };
        let body = bytes(body, body_len)?;
        if body_len > 65535 { return Err(invalid("SMP payload exceeds u16 length")); }
        if !body.is_empty() { let _: serde_cbor::Value = serde_cbor::from_slice(body)?; }
        let (_, response) = connection(c, |t| t.transceive(op, upstream::NmpGroup(group), id, body))?;
        put(out, serde_cbor::to_vec(&response)?)
    })
}

/// List images; response must include boot_version.
#[no_mangle]
pub unsafe extern "C" fn mc_image_list_json(c: *mut McConnection, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::list(t))?; json(out, value) })
}

/// Reset the device.
#[no_mangle]
pub unsafe extern "C" fn mc_reset(c: *mut McConnection) -> McStatus {
    boundary(|| { connection(c, |t| upstream::reset(t)) })
}

/// Echo; returns a JSON string.
#[no_mangle]
pub unsafe extern "C" fn mc_echo_json(c: *mut McConnection, message: *const c_char, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::echo(t, text(message)?))?; json(out, value) })
}

/// Return task statistics.
#[no_mangle]
pub unsafe extern "C" fn mc_taskstat_json(c: *mut McConnection, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::taskstat(t))?; json(out, value) })
}

/// Return device buffer parameters.
#[no_mangle]
pub unsafe extern "C" fn mc_params_json(c: *mut McConnection, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::mcumgr_params(t))?; json(out, value) })
}

/// Return OS information; format may be NULL.
#[no_mangle]
pub unsafe extern "C" fn mc_os_info_json(c: *mut McConnection, format: *const c_char, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::os_info(t, opt_text(format)?))?; json(out, value) })
}

/// Return bootloader information; query may be NULL.
#[no_mangle]
pub unsafe extern "C" fn mc_bootloader_info_json(c: *mut McConnection, query: *const c_char, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::bootloader_info(t, opt_text(query)?))?; json(out, value) })
}

/// Return statistics groups.
#[no_mangle]
pub unsafe extern "C" fn mc_stat_list_json(c: *mut McConnection, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::stat_list(t))?; json(out, value) })
}

/// Read a statistics group.
#[no_mangle]
pub unsafe extern "C" fn mc_stat_read_json(c: *mut McConnection, name: *const c_char, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::stat_read(t, text(name)?))?; json(out, value) })
}

/// Return remote file metadata.
#[no_mangle]
pub unsafe extern "C" fn mc_fs_stat_json(c: *mut McConnection, path: *const c_char, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::fs_stat(t, text(path)?))?; json(out, value) })
}

/// Hash a remote file; hash_type may be NULL; offset/length=-1 omit the field.
#[no_mangle]
pub unsafe extern "C" fn mc_fs_hash_json(c: *mut McConnection, path: *const c_char, hash_type: *const c_char, offset: i64, length: i64, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::fs_hash(t, text(path)?, opt_text(hash_type)?, optional_u32(offset)?, optional_u32(length)?))?; json(out, value) })
}

/// Download remote file to local UTF-8 path; upstream may print progress.
#[no_mangle]
pub unsafe extern "C" fn mc_fs_download(c: *mut McConnection, remote_path: *const c_char, local_path: *const c_char, subsequent_timeout_ms: u32) -> McStatus {
    boundary(|| { connection(c, |t| upstream::fs_download(t, text(remote_path)?, Path::new(text(local_path)?), positive_timeout(subsequent_timeout_ms)?)) })
}

/// Upload local file; upstream may print progress.
#[no_mangle]
pub unsafe extern "C" fn mc_fs_upload(c: *mut McConnection, local_path: *const c_char, remote_path: *const c_char, subsequent_timeout_ms: u32) -> McStatus {
    boundary(|| { connection(c, |t| upstream::fs_upload(t, Path::new(text(local_path)?), text(remote_path)?, positive_timeout(subsequent_timeout_ms)?)) })
}

/// Read setting; max_size=-1 omits the field. Binary val is a JSON byte array.
#[no_mangle]
pub unsafe extern "C" fn mc_settings_read_json(c: *mut McConnection, name: *const c_char, max_size: i64, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::settings_read(t, text(name)?, optional_u32(max_size)?))?; json(out, value) })
}

/// Write a binary setting.
#[no_mangle]
pub unsafe extern "C" fn mc_settings_write(c: *mut McConnection, name: *const c_char, value: *const u8, value_len: usize) -> McStatus {
    boundary(|| { connection(c, |t| upstream::settings_write(t, text(name)?, bytes(value, value_len)?.to_vec())) })
}

/// Delete setting.
#[no_mangle]
pub unsafe extern "C" fn mc_settings_delete(c: *mut McConnection, name: *const c_char) -> McStatus {
    boundary(|| { connection(c, |t| upstream::settings_delete(t, text(name)?)) })
}

/// commit settings.
#[no_mangle]
pub unsafe extern "C" fn mc_settings_commit(c: *mut McConnection) -> McStatus {
    boundary(|| { connection(c, |t| upstream::settings_commit(t)) })
}

/// load settings.
#[no_mangle]
pub unsafe extern "C" fn mc_settings_load(c: *mut McConnection) -> McStatus {
    boundary(|| { connection(c, |t| upstream::settings_load(t)) })
}

/// save settings.
#[no_mangle]
pub unsafe extern "C" fn mc_settings_save(c: *mut McConnection) -> McStatus {
    boundary(|| { connection(c, |t| upstream::settings_save(t)) })
}

/// Execute shell command; argv_json must be a JSON array of strings.
#[no_mangle]
pub unsafe extern "C" fn mc_shell_exec_json(c: *mut McConnection, argv_json: *const c_char, out: *mut McBuffer) -> McStatus {
    boundary(|| { output(out)?; let value = connection(c, |t| upstream::shell_exec(t, serde_json::from_str::<Vec<String>>(text(argv_json)?)?))?; json(out, value) })
}

fn optional_u32(v: i64) -> Result<Option<u32>> {
    match v { -1 => Ok(None), 0..=4294967295 => Ok(Some(v as u32)), _ => Err(invalid("optional integer must be -1 or u32")) }
}
fn positive_timeout(v: u32) -> Result<u32> {
    if v == 0 { Err(invalid("timeout must be positive")) } else { Ok(v) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    #[test]
    fn required_boot_version_over_udp() {
        for payload in [
            serde_json::json!({"boot_version": "device-v1", "images": []}),
            serde_json::json!({"images": []}),
            serde_json::json!({"boot_version": 42, "images": []}),
        ] {
            let expected_status = if payload["boot_version"].is_string() { 0 } else { 2 };
            let server = UdpSocket::bind("127.0.0.1:0").unwrap();
            server.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let port = server.local_addr().unwrap().port();
            let worker = thread::spawn(move || {
                let mut request = [0u8; 4096];
                let (n, peer) = server.recv_from(&mut request).unwrap();
                assert!(n >= 8);
                assert_eq!(request[0], 8); // Upstream UDP SMP v2 read
                assert_eq!(&request[4..6], &[0, 1]);
                let body = serde_cbor::to_vec(&payload).unwrap();
                let mut response = request[..8].to_vec();
                response[0] = 9;
                response[2..4].copy_from_slice(&(body.len() as u16).to_be_bytes());
                response.extend(body);
                server.send_to(&response, peer).unwrap();
            });
            unsafe {
                let mut c = ptr::null_mut();
                assert_eq!(mc_open_udp(b"127.0.0.1\0".as_ptr().cast(), port, 2, 512, &mut c), 0);
                let mut out = McBuffer { data: ptr::null_mut(), len: 0 };
                let status = mc_image_list_json(c, &mut out);
                assert_eq!(status, expected_status);
                if status == 0 {
                    let v: serde_json::Value = serde_json::from_slice(bytes(out.data, out.len).unwrap()).unwrap();
                    assert_eq!(v["boot_version"], "device-v1");
                } else { assert_eq!(status, 2); }
                mc_buffer_free(&mut out);
                mc_close(&mut c);
            }
            worker.join().unwrap();
        }
    }
    unsafe extern "C" fn progress(offset: u64, total: u64, user: *mut c_void) {
        assert!(offset <= total);
        (*(user as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
    }
    #[test]
    fn custom_cbor_preserves_device_error() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let port = server.local_addr().unwrap().port();
        let response_body = serde_cbor::to_vec(&serde_json::json!({"err":{"group":64,"rc":7}})).unwrap();
        let expected = response_body.clone();
        let worker = thread::spawn(move || {
            let mut request = [0u8; 4096];
            let (n, peer) = server.recv_from(&mut request).unwrap();
            assert_eq!(request[0], 10); // SMP v2 write
            assert_eq!(&request[4..6], &[0, 64]);
            assert_eq!(request[7], 42);
            assert_eq!(&request[8..n], &[0xa0]); // empty CBOR map
            let mut response = request[..8].to_vec();
            response[0] = 11;
            response[2..4].copy_from_slice(&(response_body.len() as u16).to_be_bytes());
            response.extend(response_body);
            server.send_to(&response, peer).unwrap();
        });
        unsafe {
            let mut c = ptr::null_mut();
            assert_eq!(mc_open_udp(b"127.0.0.1\0".as_ptr().cast(), port, 2, 512, &mut c), 0);
            let mut out = McBuffer { data: ptr::null_mut(), len: 0 };
            assert_eq!(mc_transceive_cbor(c, 2, 64, 42, [0xa0].as_ptr(), 1, &mut out), 0);
            let response: serde_cbor::Value = serde_cbor::from_slice(bytes(out.data, out.len).unwrap()).unwrap();
            assert_eq!(response, serde_cbor::from_slice::<serde_cbor::Value>(&expected).unwrap());
            mc_buffer_free(&mut out);
            mc_close(&mut c);
        }
        worker.join().unwrap();
    }
    #[test]
    fn upload_multiple_chunks_and_progress() { unsafe {
        let path = std::env::temp_dir().join(format!("mcumgr-ffi-{}.bin", std::process::id()));
        std::fs::write(&path, vec![0x5au8; 4096]).unwrap();
        let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut c = simulated();
        assert_eq!(mc_image_upload(c, name.as_ptr(), 1, 4, 200, Some(progress),
            Arc::as_ptr(&calls) as *mut c_void), 0);
        assert!(calls.load(Ordering::SeqCst) > 1);
        mc_close(&mut c);
        std::fs::remove_file(path).unwrap();
    }}
    unsafe fn simulated() -> *mut McConnection {
        let mut c = ptr::null_mut();
        assert_eq!(mc_open_serial(b"test\0".as_ptr().cast(), 115200, 60, 128, 512, &mut c), 0);
        c
    }
    #[test]
    fn image_json_and_ownership() { unsafe {
        let mut c = simulated();
        let mut out = McBuffer { data: ptr::null_mut(), len: 0 };
        assert_eq!(mc_image_list_json(c, &mut out), 0);
        let v: serde_json::Value = serde_json::from_slice(bytes(out.data, out.len).unwrap()).unwrap();
        assert_eq!(v["boot_version"], "test version");
        assert!(v["images"].is_array());
        assert_eq!(mc_image_list_json(c, &mut out), 1);
        assert_eq!(mc_buffer_free(&mut out), 0);
        assert!(out.data.is_null());
        assert_eq!(mc_buffer_free(&mut out), 0);
        assert_eq!(mc_close(&mut c), 0);
        assert!(c.is_null());
        assert_eq!(mc_close(&mut c), 0);
    }}
    #[test]
    fn validation_and_panic_boundary() { unsafe {
        assert_eq!(mc_reset(ptr::null_mut()), 1);
        let mut out = McBuffer { data: ptr::null_mut(), len: 0 };
        assert_eq!(mc_last_error(&mut out), 0);
        assert!(std::str::from_utf8(bytes(out.data, out.len).unwrap()).unwrap().contains("NULL"));
        mc_buffer_free(&mut out);
        assert_eq!(boundary(|| -> Result<()> { panic!("test"); }), 3);
        let mut c = simulated();
        let status = connection(c, |_| Ok(mc_reset(c))).unwrap();
        assert_eq!(status, 1);
        assert_eq!(mc_transceive_cbor(c, 99, 1, 0, ptr::null(), 0, &mut out), 1);
        mc_close(&mut c);
    }}
}
