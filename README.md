# mcumgr-client — C/C++ SDK

The `develop-cpp-fii` branch provides a Rust MCUmgr client and an independent C ABI SDK for C and C++ applications. It follows [vouch-opensource/mcumgr-client](https://github.com/vouch-opensource/mcumgr-client), retaining the mandatory `boot_version` string in image-list responses.

The SDK supports serial, UDP and BLE connections, firmware image management, device information, file transfers, statistics, settings, shell commands and custom MCUmgr groups.

## Download

Download the SDK from this fork's [Releases page](https://github.com/RichardWei/mcumgr-client/releases). Choose a release whose tag starts with `c-sdk-`.

| Platform | SDK ZIP | Static library | Header |
| --- | --- | --- | --- |
| Windows x86_64, MSVC | `mcumgr-windows-x86_64-<version>.zip` | `lib/mcumgr.lib` | `include/mcumgr.h` |
| macOS x86_64, Intel | `mcumgr-macos-x86_64-<version>.zip` | `lib/libmcumgr.a` | `include/mcumgr.h` |

Both packages include independent C/C++ CMake examples, documentation, license, build commit, toolchain information and resolved dependency versions. The Release also contains `SHA256SUMS.txt`.

Prebuilt SDK consumers do **not** need Rust or Cargo. They need a compatible C/C++ compiler and platform SDK. Windows packages target MSVC with the dynamic CRT (`/MD`); MinGW compatibility is not guaranteed. The macOS package is Intel-only.

These SDK ZIPs contain the static library and headers, not the CLI executable. Build the CLI separately if needed.

## C/C++ quick start

Extract the SDK ZIP and build its included examples from the extracted SDK directory.

Windows, with Visual Studio C++ build tools and CMake:

```powershell
cmake -S examples -B build -A x64 "-DMCUMGR_ROOT=$((Get-Location).Path)"
cmake --build build --config Release
ctest --test-dir build -C Release --output-on-failure
```

macOS, with Xcode command-line tools and CMake:

```sh
cmake -S examples -B build -DMCUMGR_ROOT="$PWD" -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release
ctest --test-dir build -C Release --output-on-failure
```

The examples use the reserved serial device name `test`, which selects the upstream simulator. They do not connect to physical hardware.

For your own application:

1. Include `mcumgr.h` and add the SDK's `include/` directory to your include paths.
2. Link the static library for your platform and the required native system libraries/frameworks.
3. Follow the imported-library configuration in the [CMake example](ffi/examples/CMakeLists.txt). The SDK's `native-static-libs.txt` records Rust's native linker dependencies.
4. Open a connection, call the command functions, release returned buffers, then close the connection.

For example, this C program queries the simulator and prints its image-list JSON:

```c
#include "mcumgr.h"
#include <stdio.h>

int main(void) {
    McConnection *connection = NULL;
    McBuffer result = {NULL, 0};

    if (mc_open_serial("test", 115200, 60, 128, 512, &connection) != MC_OK)
        return 1;

    McStatus status = mc_image_list_json(connection, &result);
    if (status == MC_OK) {
        fwrite(result.data, 1, result.len, stdout);
        fputc('\n', stdout);
    } else if (mc_last_error(&result) == MC_OK) {
        fwrite(result.data, 1, result.len, stderr);
        fputc('\n', stderr);
    }

    mc_buffer_free(&result);
    mc_close(&connection);
    return status == MC_OK ? 0 : 1;
}
```

For a physical device, replace `"test"` with its serial port, such as `"COM3"` or `"/dev/ttyACM0"`. The firmware must return a string `boot_version` in its image-list response.

See the [C example](ffi/examples/example.c), [C++ example](ffi/examples/example.cpp), [generated header](ffi/include/mcumgr.h) and [full SDK guide](ffi/README.md).

## C API

The SDK uses opaque connection handles and synchronous command calls. It uses the current upstream transport model; the old `rust_*` exports from `develop` are not retained.

| Capability | C entry points |
| --- | --- |
| Connections | `mc_open_serial`, `mc_open_udp`, `mc_open_ble`, `mc_close`, `mc_set_timeout` |
| Serial discovery | `mc_serial_ports_json` |
| Firmware images | `mc_image_list_json`, `mc_image_upload`, `mc_image_test`, `mc_image_erase` |
| Device management | `mc_reset`, `mc_echo_json`, `mc_taskstat_json`, `mc_params_json`, `mc_os_info_json`, `mc_bootloader_info_json` |
| Files | `mc_fs_download`, `mc_fs_upload`, `mc_fs_stat_json`, `mc_fs_hash_json` |
| Statistics | `mc_stat_list_json`, `mc_stat_read_json` |
| Settings | `mc_settings_read_json`, `mc_settings_write`, `mc_settings_delete`, `mc_settings_commit`, `mc_settings_load`, `mc_settings_save` |
| Shell | `mc_shell_exec_json` |
| Custom groups | `mc_transceive_cbor` |
| Diagnostics and memory | `mc_last_error`, `mc_buffer_free` |

API rules:

- Create, use and close each connection on the same thread. Do not use a connection concurrently or reenter it from a callback.
- Initialize handle outputs to `NULL` and buffer outputs to `{NULL, 0}`. Free a returned buffer before reusing it as an output.
- Returned buffers are length-delimited, not NUL-terminated. Release them with `mc_buffer_free()`, never `free()` or `delete`.
- Input strings are NUL-terminated UTF-8. Input byte buffers use a pointer and length.
- `_json` functions return JSON bytes. Custom commands accept and return CBOR bytes.
- Status codes are `MC_OK` (0), `MC_INVALID_ARGUMENT` (1), `MC_OPERATION_FAILED` (2) and `MC_PANIC` (3). Retrieve diagnostics with `mc_last_error()` immediately after a failure.
- Image upload accepts a synchronous progress callback and user pointer. The SDK follows upstream retry and chunking behavior.

Optional fields, callback lifetimes and transport limits are documented in [mcumgr.h](ffi/include/mcumgr.h) and the [SDK guide](ffi/README.md).

## Device compatibility

`ImageStateRsp` retains this branch-specific field:

```rust
pub boot_version: String,
```

It is required: a missing or non-string `boot_version` makes image-list parsing fail. This requirement applies to both the CLI and the C SDK. The separate OS bootloader-info command does not replace it.

The serial framing and existing Image/Reset command IDs are retained from the old `develop` implementation. The upstream update changes upload chunk sizing, retry conditions and retry sequence numbers. Its subsequent timeout convention is **200 ms**, compared with the old C wrapper's **4000 ms**. Pass 4000 explicitly when your device needs the longer timeout.

UDP, BLE and additional command groups require matching firmware support. Upstream error helpers do not comprehensively interpret SMP v2 nested `err` responses; custom commands return these fields for the caller to inspect.

CI verifies host-side behavior with simulated serial and loopback UDP tests. It does not establish physical-device compatibility. See the [compatibility report](ffi/COMPATIBILITY.md) for the remaining hardware checks.

## CLI

Build the root package to obtain `mcumgr-client` (`mcumgr-client.exe` on Windows):

```sh
cargo build --locked --release
```

The executable is written to `target/release/`. Rust is needed only to build from source. Linux builds also require D-Bus development files and `pkg-config` for the upstream BLE dependency.

Examples below assume the executable is on `PATH`; in PowerShell, prefix a local executable with `.\`.

```sh
# Serial
mcumgr-client -d COM3 list
mcumgr-client -d COM3 -u 4000 upload firmware.bin --slot 1

# UDP (default port: 1337)
mcumgr-client --host 192.0.2.1 list

# BLE
mcumgr-client --ble-name Zephyr list

# Inspect commands and options
mcumgr-client --help
mcumgr-client upload --help
```

| Command group | CLI commands |
| --- | --- |
| Images | `list`, `upload`, `test`, `erase` |
| Device | `reset`, `echo`, `taskstat`, `mcumgr-params`, `os-info`, `bootloader-info` |
| Files | `fs-download`, `fs-upload`, `fs-stat`, `fs-hash` |
| Statistics | `stat-list`, `stat-read` |
| Settings | `settings-read`, `settings-write`, `settings-delete`, `settings-commit`, `settings-load`, `settings-save` |
| Shell/custom | `shell`, `custom`, device-specific `hwid` and `hcdf-info` |

CLI defaults: 115200 baud, 60-second initial timeout, 200-ms subsequent timeout, 4 retries, serial line length 128 and request MTU 512. BLE defaults to a 10-second scan timeout and payload MTU 244. Device buffer limits still apply.

Use `-s` for silent mode and check the process exit code; errors are also suppressed. An upload filename containing `slot1` or `slot3` can override the requested image slot, following upstream behavior.

## Build and test the SDK from source

The C ABI is a separate Cargo package under `ffi/`, depending on the root library by path. It does not introduce a root Cargo workspace or duplicate the protocol implementation.

```sh
python ffi/tools/generate_header.py --check
cargo test --locked
cargo test --manifest-path ffi/Cargo.toml
cargo build --manifest-path ffi/Cargo.toml --release
```

The static library is written to `ffi/target/release/`. To regenerate the header after changing exported signatures:

```sh
python ffi/tools/generate_header.py
```

For an explicit target, pass `--target x86_64-pc-windows-msvc` or `--target x86_64-apple-darwin`; the library then appears under `ffi/target/<target>/release/`. Native platform toolchains are required. The [C SDK workflow](.github/workflows/ffi.yaml) performs these builds on GitHub-hosted runners.

## Automated releases

Only a push to `develop-cpp-fii` triggers the C SDK workflow:

1. Build and test on Windows x86_64 and macOS Intel.
2. Check the generated header and compile, link and run independent C/C++ consumers.
3. Package each SDK and upload Actions artifacts.
4. After both platforms succeed, verify ZIP contents, versions and commit IDs; generate `SHA256SUMS.txt`.
5. Create a tag pointing to the built commit and publish both ZIPs and checksums in one Release.

Tag format:

```text
c-sdk-<version>-<UTC YYYYMMDD>-g<12-character commit>
```

SDK version is defined in [ffi/VERSION](ffi/VERSION) and must match [ffi/Cargo.toml](ffi/Cargo.toml). No manual tag push is required. A documentation-only push to this branch also triggers a build and Release because there is no path filter.

The SDK workflow does not publish Linux, ARM64, shared libraries or CLI executables. The upstream CLI Release workflow is separate.

## Upstream maintenance

The original `develop` branch preserves the previous implementation. Work on `develop-cpp-fii` is based on upstream `main`, with the required `boot_version` field and simulator fixture retained in the root source. C bindings, examples and SDK documentation live under `ffi/`; SDK CI lives in its own workflow.

When updating, rebase this branch onto `upstream/main`, preserve the required response field and fixture, and adapt the independent C wrapper to public Rust API changes. Verify both platform builds and device behavior before relying on a new SDK.

## License

[Apache License 2.0](LICENSE).
