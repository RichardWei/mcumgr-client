# MCUmgr C SDK

This independent Rust package wraps the upstream public API. The root package remains upstream-compatible except for the mandatory `ImageStateRsp.boot_version: String` and simulator fixture. Old `rust_*` symbols are intentionally not supported.

## Download and use

The **C SDK** Actions workflow runs only on pushes to `develop-cpp-fii`. Download either `mcumgr-windows-x86_64` or `mcumgr-macos-x86_64`, then extract its SDK ZIP. Each SDK includes:

- `include/mcumgr.h`
- `lib/mcumgr.lib` (Windows MSVC x64) or `lib/libmcumgr.a` (Intel macOS)
- Independent C and C++ CMake examples, documentation, license, commit, dependency and toolchain information
- `native-static-libs.txt`, the actual Rust native dependency linker report

The libraries contain Rust code and its compiled dependencies. Consumers need a compatible native compiler, platform SDK, and the documented system libraries/frameworks; they do not need Cargo or Rust. Windows uses the MSVC dynamic CRT (/MD); MinGW ABI compatibility is not promised. macOS requires the system frameworks listed in the example. Use a deployment target compatible with the SDK build; the runner SDK is recorded in BUILD_ENVIRONMENT.txt.

```sh
cmake -S examples -B build -DMCUMGR_ROOT=/absolute/path/to/extracted/sdk -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release
ctest --test-dir build -C Release --output-on-failure
```

On Windows, run CMake with an x64 Visual Studio generator. For both platforms, the shipped examples link to the static library only; their CMake target adds native dependencies.

## API conventions

- All functions use the platform C calling convention (explicit `__cdecl` on Windows).
- Status: `MC_OK=0`, `MC_INVALID_ARGUMENT=1`, `MC_OPERATION_FAILED=2`, `MC_PANIC=3`.
- Call `mc_last_error()` immediately after an error, on the same thread, to copy its UTF-8 diagnostic. Successful operations normally clear that thread's diagnostic. Error retrieval preserves it.
- Strings must be NUL-terminated UTF-8. Byte buffers use pointer and length; NULL is accepted only for zero-length input.
- Initialize outputs to NULL or `McBuffer {NULL, 0}`. Outputs remain empty on failure. Do not pass a previously allocated nonempty output.
- Buffers are not NUL-terminated. Read exactly `len` bytes; release once with `mc_buffer_free()`. Never use `free/delete`, alter fields, or duplicate ownership.
- Connection handles are library owned. Close with `mc_close(&connection)`, which resets the pointer. Do not copy ownership, reuse stale handles, or pass invalid pointers.
- Create, operate, and close each connection on the same thread. Do not access it concurrently. Calls and callbacks are synchronous. Reentry or close from an upload callback is rejected. Callback pointers and user data need remain valid only for the call duration.
- Query functions ending in `_json` return JSON bytes. OS info/echo are JSON strings; other queries return their upstream response objects. Binary fields serialize as JSON arrays, except upstream image hashes, which use upstream's hex formatting.
- `mc_transceive_cbor()` sends raw CBOR for arbitrary group/command IDs and returns response-body CBOR. It does not interpret device error fields.
- Signed optional integers use -1 to omit a field. Image confirmation uses -1/0/1 for omit/false/true. Nullable strings are documented in the header.
- Upload callbacks report acknowledged offset and total. File-system operations retain upstream console progress behavior and do not add a callback.
- Functions mirror upstream behavior, including retry policy and error parsing limitations. A successful transport call does not universally mean a device command succeeded; inspect custom response error fields and shell `rc`.

## Defaults and transport settings

Open functions take explicit settings. Use upstream CLI defaults when desired:

| Setting | Value |
|---|---|
| Serial baud / initial timeout / line / MTU | 115200 / 60 seconds / 128 / 512 |
| UDP port / initial timeout / MTU | 1337 / 60 seconds / 512 |
| BLE scan / response timeout / MTU | 10 seconds / 60 seconds / 244 |
| Image retries / subsequent timeout | 4 / 200 milliseconds |

The new ABI intentionally does not retain develop's previous 4000 ms subsequent timeout. Pass 4000 explicitly if your device requires it. Upload/file operations retain their changed transport timeout for subsequent calls; use `mc_set_timeout()` to change it. IPv6 UDP hosts must be bracketed, e.g. `[::1]`. BLE uses upstream discovery and may require platform Bluetooth permissions.

The wrapper rejects serial lines below 8, serial/UDP MTUs below 128, BLE MTUs outside 128..512, zero timeouts, and empty image files before calling upstream. These are conservative host-side constraints, not a guarantee that the device supports the selected MTU.

## Rebuild (CI or a machine with Rust)

```sh
python ffi/tools/generate_header.py
cargo test --locked
cargo test --manifest-path ffi/Cargo.toml
cargo build --release
cargo rustc --manifest-path ffi/Cargo.toml --release -- --print native-static-libs
```

The header generator reads the restricted C export signatures directly and fails for unrecognized types. CI checks that the tracked header matches the source. The FFI package has its own dependency resolution; each artifact includes the exact resolved Cargo.lock. No root workspace or protocol implementation is added.

## Updating upstream

Keep local upstream changes limited to mandatory boot_version and its fixture. Rebase the working branch onto upstream/main, resolve the root package in upstream's favor except for those fields, and adapt this independent package if public Rust APIs change. The original develop branch retains the old implementation and history.

