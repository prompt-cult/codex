//! The WebAssembly export layer.
//!
//! A thin shell over the pure API in `lib.rs`, so every behaviour is reachable
//! from a native test and nothing interesting lives behind `cfg(wasm32)`.
//! See `ABI.md` — Step machine — for the contract these exports implement.
//!
//! Buffers are owned explicitly: the host allocates with `tb_alloc`, writes
//! bytes, and passes pointer plus length. Results are returned as a packed
//! `(ptr << 32) | len`, and remain valid until the next call that produces
//! output.

use crate::{load, Evaluator};
use serde_json::Value;
use std::cell::RefCell;

thread_local! {
    static EVALUATOR: RefCell<Option<Evaluator>> = const { RefCell::new(None) };
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_ERROR: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn pack(ptr: usize, len: usize) -> i64 {
    (((ptr as u64) << 32) | (len as u64 & 0xFFFF_FFFF)) as i64
}

fn set_error(message: &str) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = message.as_bytes().to_vec());
}

/// Publish `json` as the current output buffer and return its packed handle.
fn publish(json: &Value) -> i64 {
    let bytes = serde_json::to_vec(json).unwrap_or_else(|_| b"{}".to_vec());
    OUTPUT.with(|slot| {
        let mut buffer = slot.borrow_mut();
        *buffer = bytes;
        pack(buffer.as_ptr() as usize, buffer.len())
    })
}

/// # Safety
/// The host must treat the returned pointer as owning `len` bytes and release
/// it with `tb_dealloc`.
#[no_mangle]
pub extern "C" fn tb_alloc(len: i32) -> i32 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(0) as usize);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr as i32
}

/// # Safety
/// `ptr` and `len` must come from a previous `tb_alloc`.
#[no_mangle]
pub unsafe extern "C" fn tb_dealloc(ptr: i32, len: i32) {
    if ptr == 0 || len <= 0 {
        return;
    }
    drop(Vec::from_raw_parts(ptr as *mut u8, 0, len as usize));
}

/// # Safety
/// Both pointers must name valid UTF-8 buffers of the given lengths.
unsafe fn read(ptr: i32, len: i32) -> Result<String, String> {
    if ptr == 0 || len < 0 {
        return Err("host passed a null or negatively sized buffer".to_string());
    }
    let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    String::from_utf8(slice.to_vec()).map_err(|e| format!("host buffer is not valid UTF-8: {e}"))
}

/// Load and validate a graph. Returns 0 on success; on failure the message is
/// available through `tb_last_error`.
///
/// # Safety
/// The four arguments must name valid buffers.
#[no_mangle]
pub unsafe extern "C" fn tb_start(ir_ptr: i32, ir_len: i32, cfg_ptr: i32, cfg_len: i32) -> i32 {
    let ir = match read(ir_ptr, ir_len) {
        Ok(s) => s,
        Err(e) => {
            set_error(&e);
            return 1;
        }
    };
    let cfg = match read(cfg_ptr, cfg_len) {
        Ok(s) => s,
        Err(e) => {
            set_error(&e);
            return 2;
        }
    };
    match load(&ir, &cfg) {
        Ok(evaluator) => {
            EVALUATOR.with(|slot| *slot.borrow_mut() = Some(evaluator));
            0
        }
        Err(message) => {
            set_error(&message);
            3
        }
    }
}

/// Advance one step. Pass `(0, 0)` for the first call, otherwise the
/// `tool.response` envelope. Returns a packed pointer/length naming either a
/// `tool.request` or a `run.finished`. On error the packed length is 0 and the
/// message is available through `tb_last_error`.
///
/// # Safety
/// `ptr` and `len` must name a valid buffer, or be `(0, 0)`.
#[no_mangle]
pub unsafe extern "C" fn tb_step(ptr: i32, len: i32) -> i64 {
    let response = if ptr == 0 || len == 0 {
        None
    } else {
        match read(ptr, len) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) => Some(v),
                Err(e) => {
                    set_error(&format!("tool response is not valid JSON: {e}"));
                    return pack(0, 0);
                }
            },
            Err(e) => {
                set_error(&e);
                return pack(0, 0);
            }
        }
    };

    EVALUATOR.with(|slot| {
        let mut held = slot.borrow_mut();
        let Some(evaluator) = held.as_mut() else {
            set_error("tb_step was called before tb_start");
            return pack(0, 0);
        };
        match evaluator.step(response.as_ref()) {
            Ok(envelope) => publish(&envelope),
            Err(e) => {
                set_error(&e.message);
                pack(0, 0)
            }
        }
    })
}

/// The last error message, as a packed pointer/length.
#[no_mangle]
pub extern "C" fn tb_last_error() -> i64 {
    LAST_ERROR.with(|slot| {
        let buffer = slot.borrow();
        pack(buffer.as_ptr() as usize, buffer.len())
    })
}
