// lib.rs

// 导入必要的模块和依赖
mod default;
mod image;
mod nmp_hdr;
mod test_serial_port;
mod transfer;

pub use crate::default::reset;
pub use crate::image::{erase, list, test, upload};
pub use crate::transfer::SerialSpecs;

// 引入所需的外部 crate
// use anyhow::anyhow; // 仅保留需要的部分
use clap::Parser;
// use hex;
// use log::error; // 仅保留需要的部分
use serde_json;
// use simplelog::{ColorChoice, Config, SimpleLogger, TermLogger, TerminalMode}; // 保留必要的部分
use std::convert::TryInto;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::path::PathBuf;
use std::ptr;

/// 定义进度回调函数类型
pub type ProgressCallback = extern "C" fn(offset: u64, total: u64);

/// 定义命令行参数解析的结构体
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// 设备名称
    #[arg(short, long, default_value = "")]
    pub device: String,

    /// 是否启用详细模式
    #[arg(short, long)]
    pub verbose: bool,

    /// 初始超时时间（秒）
    #[arg(short = 't', long = "initial_timeout", default_value_t = 60)]
    pub initial_timeout_s: u32,

    /// 后续超时时间（毫秒）
    #[arg(short = 'u', long = "subsequent_timeout", default_value_t = 200)]
    pub subsequent_timeout_ms: u32,

    /// 每个数据包的重试次数
    #[arg(long, default_value_t = 4)]
    pub nb_retry: u32,

    /// 每行的最大长度
    #[arg(short, long, default_value_t = 128)]
    pub linelength: usize,

    /// 每个请求的最大长度
    #[arg(short, long, default_value_t = 512)]
    pub mtu: usize,

    /// 波特率
    #[arg(short, long, default_value_t = 115_200)]
    pub baudrate: u32,
}

impl From<&Cli> for SerialSpecs {
    fn from(cli: &Cli) -> SerialSpecs {
        SerialSpecs {
            device: cli.device.clone(),
            initial_timeout_s: cli.initial_timeout_s,
            subsequent_timeout_ms: cli.subsequent_timeout_ms,
            nb_retry: cli.nb_retry,
            linelength: cli.linelength,
            mtu: cli.mtu,
            baudrate: cli.baudrate,
        }
    }
}

/// 为 `SerialSpecs` 实现 `Default` trait
impl Default for SerialSpecs {
    fn default() -> Self {
        SerialSpecs {
            device: String::new(),
            initial_timeout_s: 60,
            subsequent_timeout_ms: 200,
            nb_retry: 4,
            linelength: 128,
            mtu: 512,
            baudrate: 115_200,
        }
    }
}

/// 列出指定设备上的所有可用插槽
#[no_mangle]
pub extern "C" fn rust_list(device: *const c_char) -> *mut c_char {
    if device.is_null() {
        return ptr::null_mut();
    }

    // 将 C 字符串转换为 Rust 字符串
    let device_str = unsafe { CStr::from_ptr(device) };
    let device_name = match device_str.to_str() {
        Ok(s) => s,
        Err(_) => {
            return ptr::null_mut();
        }
    };

    // 创建 SerialSpecs，包含指定的设备名称
    let specs = SerialSpecs {
        device: device_name.to_string(),
        ..SerialSpecs::default()
    };

    // 调用 list 函数，获取插槽信息
    match list(&specs) {
        Ok(v) => {
            let json = serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string());

            CString::new(json).unwrap().into_raw()
        }
        Err(_e) => ptr::null_mut(),
    }
}

/// 上传文件到设备
#[no_mangle]
pub extern "C" fn rust_upload(
    device: *const c_char,
    filename: *const c_char,
    slot: c_uint,
    callback: Option<ProgressCallback>,
) -> c_int {
    if device.is_null() || filename.is_null() {
        return -1;
    }

    let device_str = unsafe { CStr::from_ptr(device) };
    let device_name = match device_str.to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let file_str = unsafe { CStr::from_ptr(filename) };
    let filename_str = match file_str.to_str() {
        Ok(s) => s,
        Err(_) => return -3,
    };

    let specs = SerialSpecs {
        device: device_name.to_string(),
        ..SerialSpecs::default()
    };

    let slot_u8: u8 = match slot.try_into() {
        Ok(s) => s,
        Err(_) => return -4, // slot 超出 u8 范围
    };

    match upload(
        &specs,
        &PathBuf::from(filename_str),
        slot_u8,
        callback.map(|cb| {
            move |offset, total| {
                cb(offset, total);
            }
        }),
    ) {
        Ok(_) => 0,
        Err(_) => -5,
    }
}

/// 重置设备
#[no_mangle]
pub extern "C" fn rust_reset(device: *const c_char) -> c_int {
    if device.is_null() {
        return -1;
    }

    let device_str = unsafe { CStr::from_ptr(device) };
    let device_name = match device_str.to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let specs = SerialSpecs {
        device: device_name.to_string(),
        ..SerialSpecs::default()
    };

    match reset(&specs) {
        Ok(_) => 0,
        Err(_) => -3,
    }
}

// /// 测试设备
// #[no_mangle]
// pub extern "C" fn rust_test(
//     device: *const c_char,
//     hash: *const c_char,
//     confirm: c_int,
// ) -> c_int {
//     if device.is_null() || hash.is_null() {
//         return -1;
//     }

//     let device_str = unsafe { CStr::from_ptr(device) };
//     let device_name = match device_str.to_str() {
//         Ok(s) => s,
//         Err(_) => return -2,
//     };

//     let hash_str = unsafe { CStr::from_ptr(hash) };
//     let hash_str = match hash_str.to_str() {
//         Ok(s) => s,
//         Err(_) => return -3,
//     };

//     let hash_bytes = match hex::decode(hash_str) {
//         Ok(bytes) => bytes,
//         Err(_) => return -4,
//     };

//     let specs = SerialSpecs {
//         device: device_name.to_string(),
//         ..SerialSpecs::default()
//     };

//     match test(&specs, hash_bytes, Some(confirm != 0)) {
//         Ok(_) => 0,
//         Err(_) => -5,
//     }
// }

// /// 擦除设备上的指定插槽
// #[no_mangle]
// pub extern "C" fn rust_erase(device: *const c_char, slot: c_uint) -> c_int {
//     if device.is_null() {
//         return -1;
//     }

//     let device_str = unsafe { CStr::from_ptr(device) };
//     let device_name = match device_str.to_str() {
//         Ok(s) => s,
//         Err(_) => return -2,
//     };

//     let specs = SerialSpecs {
//         device: device_name.to_string(),
//         ..SerialSpecs::default()
//     };

//     match erase(&specs, Some(slot)) {
//         Ok(_) => 0,
//         Err(_) => -3,
//     }
// }

/// 添加一个清理函数，用于释放从 Rust 返回的字符串
#[no_mangle]
pub extern "C" fn rust_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(s);
    }
}

/// 新增函数：列出所有可用的串口设备
#[no_mangle]
pub extern "C" fn rust_list_ports() -> *mut c_char {
    // 调用 serialport::available_ports()
    match serialport::available_ports() {
        Ok(ports) => {
            // 将串口信息转换为可序列化的结构体
            let port_names: Vec<String> = ports.iter().map(|p| p.port_name.clone()).collect();

            // 序列化为 JSON
            let json =
                serde_json::to_string_pretty(&port_names).unwrap_or_else(|_| "[]".to_string());

            // 转换为 C 字符串并返回
            CString::new(json).unwrap().into_raw()
        }
        Err(_e) => ptr::null_mut(),
    }
}
